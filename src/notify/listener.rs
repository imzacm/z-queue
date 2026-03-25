use core::pin::Pin;
use core::sync::atomic::Ordering;
use core::task::{Context, Poll};

use parking_lot_core::DEFAULT_PARK_TOKEN;

use super::Notify;
use super::waker_queue::WakerTicket;

/// A listener that was created from a [`Notify`].
///
/// Supports both blocking (`.wait()`) and async (`.await`) usage.
#[derive(Debug)]
pub struct NotifyListener<'a> {
    notify: &'a Notify,
    /// The epoch snapshot taken when this listener was created.
    epoch: u64,
    key: usize,
    waker_node_ticket: Option<WakerTicket>,
}

impl<'a> NotifyListener<'a> {
    pub(super) fn new(notify: &'a Notify, epoch: u64) -> Self {
        Self { notify, epoch, key: notify.parking_key(), waker_node_ticket: None }
    }

    #[inline(always)]
    pub fn notification(&self) -> &'a Notify {
        self.notify
    }

    #[inline(always)]
    pub fn is_notification(&self, notify: &Notify) -> bool {
        core::ptr::eq(self.notify, notify)
    }

    /// Returns `true` if a notification has occurred since this listener was created.
    #[inline(always)]
    pub fn is_notified(&self) -> bool {
        self.notify.epoch.load(Ordering::Acquire) != self.epoch
    }

    /// Blocks the current thread until a notification arrives.
    pub fn wait(self) {
        if self.is_notified() {
            return;
        }

        for _ in 0..64 {
            if self.is_notified() {
                return;
            }
            core::hint::spin_loop();
        }

        self.notify.parked_count.fetch_add(1, Ordering::SeqCst);

        loop {
            // SAFETY: We use the epoch address as a stable key.
            // The validate closure re-checks under the parking_lot_core's
            // internal lock, preventing missed wakeups.
            unsafe {
                parking_lot_core::park(
                    self.key,
                    || !self.is_notified(), // validate: should we actually park?
                    || {},                  // before_sleep: nothing to do
                    |_, _| {},              // timed_out: not using timeouts
                    DEFAULT_PARK_TOKEN,
                    None, // no timeout
                );
            }

            if self.is_notified() {
                self.notify.parked_count.fetch_sub(1, Ordering::Relaxed);
                return;
            }

            // Spurious wakeup — loop back and park again.
        }
    }
}

impl<'a> Future for NotifyListener<'a> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.is_notified() {
            return Poll::Ready(());
        }

        let this = self.as_mut().get_mut();

        // This creates a memory barrier, so even if we don't need the lock (early return), skipping
        // the lock can cause deadlocks (rare).
        let mut queue = this.notify.async_wakers.lock();

        if this.is_notified() {
            // We already hold the lock, clean up now so Drop doesn't have to re-lock.
            if let Some(ticket) = this.waker_node_ticket.take()
                && queue.remove(ticket)
            {
                self.notify.async_count.fetch_sub(1, Ordering::Relaxed);
            }

            return Poll::Ready(());
        }

        if let Some(ticket) = this.waker_node_ticket {
            let node = queue.node_mut(ticket.index());

            if node.generation() == ticket.generation() {
                if node.waker().is_none_or(|w| !w.will_wake(cx.waker())) {
                    *node.waker_mut() = Some(cx.waker().clone());
                }
            } else {
                // Our slot was popped and recycled by a previous wakeup. We must re-enqueue
                // ourselves to prevent a lost wakeup.
                this.waker_node_ticket = Some(queue.push(cx.waker().clone()));
                this.notify.async_count.fetch_add(1, Ordering::SeqCst);
            }
        } else {
            // First time being polled.
            this.waker_node_ticket = Some(queue.push(cx.waker().clone()));
            this.notify.async_count.fetch_add(1, Ordering::SeqCst);
        }

        if this.is_notified() {
            if let Some(ticket) = this.waker_node_ticket.take()
                && queue.remove(ticket)
            {
                self.notify.async_count.fetch_sub(1, Ordering::Relaxed);
            }
            return Poll::Ready(());
        }

        Poll::Pending
    }
}

impl<'a> Drop for NotifyListener<'a> {
    fn drop(&mut self) {
        if let Some(ticket) = self.waker_node_ticket.take()
            && self.notify.async_wakers.lock().remove(ticket)
        {
            self.notify.async_count.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

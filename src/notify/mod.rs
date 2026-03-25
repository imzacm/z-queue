mod listener;
mod select;
mod waker_queue;

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use core::task::Waker;

use crossbeam_utils::CachePadded;
use parking_lot::Mutex;
use parking_lot_core::DEFAULT_UNPARK_TOKEN;

pub use self::listener::NotifyListener;
pub use self::select::select_blocking;
use self::waker_queue::WakerQueue;

const ASYNC_CAPACITY: usize = 8;

/// A lightweight notification primitive supporting both blocking and async waiters.
///
/// Designed as a drop-in replacement for `event_listener::Event`, optimised for
/// the check → listen → check → wait pattern used throughout this crate.
///
/// The implementation uses a monotonically increasing "epoch" counter.
/// A [`NotifyListener`] captures the epoch at creation time; it only completes
/// once the epoch has advanced past that snapshot, which means a notification
/// was fired *after* the listener was registered.
#[derive(Debug)]
pub struct Notify {
    epoch: CachePadded<AtomicU64>,
    /// Hint: number of threads currently parked via `parking_lot_core`.
    /// Used to skip the expensive `unpark_*` call when no one is waiting.
    parked_count: CachePadded<AtomicU32>,
    async_count: CachePadded<AtomicU32>,
    async_wakers: CachePadded<Mutex<WakerQueue<ASYNC_CAPACITY>>>,
}

impl Default for Notify {
    fn default() -> Self {
        Self::new()
    }
}

impl Notify {
    pub fn new() -> Self {
        Self {
            epoch: CachePadded::new(AtomicU64::new(0)),
            parked_count: CachePadded::new(AtomicU32::new(0)),
            async_count: CachePadded::new(AtomicU32::new(0)),
            async_wakers: CachePadded::new(Mutex::new(WakerQueue::new())),
        }
    }

    /// Creates a listener that captures the current epoch.
    ///
    /// Typical use:
    /// ```ignore
    /// let listener = notify.listener();
    /// // re-check your condition here
    /// listener.wait();   // or  listener.await
    /// ```
    #[inline]
    pub fn listener(&self) -> NotifyListener<'_> {
        let epoch = self.epoch.load(Ordering::Acquire);
        NotifyListener::new(self, epoch)
    }

    /// Wake up to `n` waiting tasks/threads.
    ///
    /// Semantics: advances the epoch, then wakes at most `n` waiters
    /// (a mix of async wakers and parked threads).
    pub fn notify(&self, n: usize) {
        if n == 0 {
            return;
        }
        self.epoch.fetch_add(1, Ordering::SeqCst);

        let mut remaining = n;

        // Wake async waiters first (cheaper than syscalls).
        remaining = self.wake_async(remaining);

        // Wake blocked threads only if any are actually parked.
        if remaining > 0 {
            self.wake_blocking(remaining);
        }
    }

    /// Returns remaining.
    fn wake_async(&self, mut remaining: usize) -> usize {
        const BATCH_SIZE: usize = 32;

        if self.async_count.load(Ordering::SeqCst) == 0 {
            return remaining;
        }

        loop {
            let mut popped = 0;

            // Bypass the 512-byte memset overhead completely.
            let mut wakers: [MaybeUninit<Waker>; BATCH_SIZE] =
                [const { MaybeUninit::uninit() }; BATCH_SIZE];

            {
                let mut queue = self.async_wakers.lock();
                while remaining > 0 && popped < BATCH_SIZE {
                    let Some(waker) = queue.pop_and_take() else { break };
                    wakers[popped].write(waker);
                    popped += 1;
                    remaining -= 1;
                }
            }

            if popped == 0 {
                break;
            }

            self.async_count.fetch_sub(popped as u32, Ordering::SeqCst);

            for waker in &mut wakers[..popped] {
                // SAFETY: We explicitly initialized exactly `popped` elements
                // inside the mutex lock above.
                unsafe {
                    waker.assume_init_read().wake();
                }
            }

            if remaining == 0 {
                break;
            }
        }

        remaining
    }

    /// Returns remaining.
    fn wake_blocking(&self, n: usize) -> usize {
        if self.parked_count.load(Ordering::SeqCst) == 0 {
            return n;
        }

        let mut remaining = n;

        let key = self.parking_key();
        if remaining == usize::MAX {
            let unparked = unsafe { parking_lot_core::unpark_all(key, DEFAULT_UNPARK_TOKEN) };
            remaining = remaining.saturating_sub(unparked);
            return remaining;
        }

        let mut unparked = 0;
        unsafe {
            parking_lot_core::unpark_filter(
                key,
                |_| {
                    if unparked < n {
                        unparked += 1;
                        parking_lot_core::FilterOp::Unpark
                    } else {
                        parking_lot_core::FilterOp::Stop
                    }
                },
                |_| DEFAULT_UNPARK_TOKEN,
            );
        }

        remaining - unparked
    }

    /// The address used as the parking key.
    #[inline]
    fn parking_key(&self) -> usize {
        core::ptr::from_ref(&self.epoch) as usize
    }
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn test_async() {
        let notify = Arc::new(Notify::new());

        let listener = notify.listener();
        assert!(!listener.is_notified());

        let notify_clone = notify.clone();
        tokio::spawn(async move {
            notify_clone.notify(1);
        });

        listener.await;

        notify.notify(1);
        let listener = notify.listener();
        assert!(!listener.is_notified());
        notify.notify(1);
        assert!(listener.is_notified());
    }
}

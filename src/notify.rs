use alloc::vec::Vec;
use core::future::Future;
use core::mem::MaybeUninit;
use core::num::NonZeroU64;
use core::pin::Pin;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use core::task::{Context, Poll, Waker};

use crossbeam_utils::CachePadded;
use parking_lot::Mutex;
use parking_lot_core::{DEFAULT_PARK_TOKEN, DEFAULT_UNPARK_TOKEN};

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

/// A listener that was created from a [`Notify`].
///
/// Supports both blocking (`.wait()`) and async (`.await`) usage.
pub struct NotifyListener<'a> {
    notify: &'a Notify,
    /// The epoch snapshot taken when this listener was created.
    epoch: u64,
    key: usize,
    waker_node_ticket: Option<WakerTicket>,
}

impl<'a> NotifyListener<'a> {
    fn new(notify: &'a Notify, epoch: u64) -> Self {
        Self { notify, epoch, key: notify.parking_key(), waker_node_ticket: None }
    }

    /// Returns `true` if a notification has occurred since this listener was created.
    #[inline]
    fn is_notified(&self) -> bool {
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

            if node.generation == ticket.generation() {
                if node.waker.as_ref().is_none_or(|w| !w.will_wake(cx.waker())) {
                    node.waker = Some(cx.waker().clone());
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

/// An allocation-free, generational doubly-linked list node.
#[derive(Debug)]
pub struct WakerNode {
    /// Prevents the ABA problem if a slot is rapidly reused.
    generation: u32,
    waker: Option<Waker>,
    prev: u32,
    next: u32,
}

impl WakerNode {
    const NULL_NODE: u32 = u32::MAX;
}

impl Default for WakerNode {
    fn default() -> Self {
        Self {
            generation: 0,
            waker: None,
            prev: Self::NULL_NODE,
            next: Self::NULL_NODE,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct WakerTicket(NonZeroU64);

impl WakerTicket {
    #[inline(always)]
    fn new(index: u32, generation: u32) -> Self {
        // Shift index to the top 32 bits, generation to the bottom 32.
        let value = ((index as u64) << 32) | (generation as u64);
        // Add 1 so the bit pattern is never zero, allowing Option to use 0 as `None`
        let value = unsafe { NonZeroU64::new_unchecked(value + 1) };
        Self(value)
    }

    #[inline(always)]
    fn index(self) -> u32 {
        ((self.0.get() - 1) >> 32) as u32
    }

    #[inline(always)]
    fn generation(self) -> u32 {
        (self.0.get() - 1) as u32
    }
}

/// A pre-allocated queue for async wakers supporting O(1) push, pop, and removal.
/// Spills over into a dynamic vector if the array fills up.
#[derive(Debug)]
struct WakerQueue<const ARRAY_CAPACITY: usize> {
    nodes_array: [WakerNode; ARRAY_CAPACITY],
    nodes_vec: Vec<WakerNode>,
    head: u32,
    tail: u32,
    free_head: u32,
}

impl<const ARRAY_CAPACITY: usize> WakerQueue<ARRAY_CAPACITY> {
    fn new() -> Self {
        assert!(
            ARRAY_CAPACITY < u32::MAX as usize,
            "WakerQueue capacity must be less than `u32::MAX`"
        );

        Self {
            nodes_array: core::array::from_fn(|index| WakerNode {
                generation: 0,
                waker: None,
                prev: WakerNode::NULL_NODE,
                next: if index == ARRAY_CAPACITY - 1 {
                    WakerNode::NULL_NODE
                } else {
                    (index + 1) as u32
                },
            }),
            nodes_vec: Vec::new(),
            head: WakerNode::NULL_NODE,
            tail: WakerNode::NULL_NODE,
            free_head: 0,
        }
    }

    /// Helper to seamlessly index into either the inline array or the fallback vector.
    #[inline(always)]
    fn node_mut(&mut self, idx: u32) -> &mut WakerNode {
        let idx = idx as usize;
        if idx < ARRAY_CAPACITY {
            unsafe { self.nodes_array.get_unchecked_mut(idx) }
        } else {
            &mut self.nodes_vec[idx - ARRAY_CAPACITY]
        }
    }

    /// Pushes a waker to the back of the queue. Returns an (index, generation) ticket.
    fn push(&mut self, waker: Waker) -> WakerTicket {
        // 1. Claim a free slot or allocate a new one in the vector
        let idx = if self.free_head != WakerNode::NULL_NODE {
            let idx = self.free_head;
            self.free_head = self.node_mut(idx).next;
            idx
        } else {
            let idx = (ARRAY_CAPACITY + self.nodes_vec.len()) as u32;
            self.nodes_vec.push(WakerNode::default());
            idx
        };

        // 2. Initialize the claimed node (tightly scoped for borrow checker)
        let generation = {
            let tail = self.tail;
            let node = self.node_mut(idx);
            node.waker = Some(waker);
            node.prev = tail;
            node.next = WakerNode::NULL_NODE;
            node.generation
        };

        // 3. Link it into the active queue
        if self.tail != WakerNode::NULL_NODE {
            self.node_mut(self.tail).next = idx;
        } else {
            self.head = idx;
        }
        self.tail = idx;

        WakerTicket::new(idx, generation)
    }

    /// Safely unlinks a node (used when futures are dropped).
    ///
    /// Returns true if the node was successfully removed, false if it was already popped.
    fn remove(&mut self, ticket: WakerTicket) -> bool {
        let index = ticket.index();

        // Scope the borrow to extract current state so we can mutate neighbors later
        let (node_gen, prev, next) = {
            let node = self.node_mut(index);
            (node.generation, node.prev, node.next)
        };

        // If generation mismatches, this slot was already popped and reused.
        if node_gen != ticket.generation() {
            return false;
        }

        // Invalidate the node
        {
            let node = self.node_mut(index);
            node.generation = node.generation.wrapping_add(1);
            node.waker = None;
        }

        // Unlink from neighbors
        if prev != WakerNode::NULL_NODE {
            self.node_mut(prev).next = next;
        } else {
            self.head = next;
        }

        if next != WakerNode::NULL_NODE {
            self.node_mut(next).prev = prev;
        } else {
            self.tail = prev;
        }

        // Push the slot back onto the free list
        // Note: This seamlessly chains free array slots AND free vector slots together!
        {
            let head = self.free_head;
            let node = self.node_mut(index);
            node.next = head;
            self.free_head = index;
        }

        true
    }

    /// Pops the front waker and removes it from the queue.
    fn pop_and_take(&mut self) -> Option<Waker> {
        if self.head == WakerNode::NULL_NODE {
            return None;
        }

        let idx = self.head;
        let (waker, next) = {
            let free_head = self.free_head;
            let node = self.node_mut(idx);
            node.generation = node.generation.wrapping_add(1);
            let waker = node.waker.take();
            let waker = unsafe { waker.unwrap_unchecked() };
            let next = node.next;

            // Push directly to free list
            node.next = free_head;
            (waker, next)
        };

        // Re-link queue
        self.head = next;
        if next != WakerNode::NULL_NODE {
            self.node_mut(next).prev = WakerNode::NULL_NODE;
        } else {
            self.tail = WakerNode::NULL_NODE;
        }
        self.free_head = idx;

        Some(waker)
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

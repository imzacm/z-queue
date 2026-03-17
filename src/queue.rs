use alloc::vec::Vec;
use core::num::NonZeroUsize;

use crossbeam_utils::CachePadded;
use event_listener::{Event, Listener};

use crate::container::{Container, CreateBounded, CreateUnbounded};

pub const MAX_SMALL_CAPACITY: usize = 1024;

#[derive(Debug)]
pub struct ZQueue<C> {
    container: C,
    // Notified on push.
    push_event: CachePadded<Event>,
    // Notified on pop.
    pub(crate) pop_event: CachePadded<Event>,
    // All waiters are notified on every push.
    find_waiter: CachePadded<Event>,
}

impl<C: Container> ZQueue<C> {
    pub fn new(container: C) -> Self {
        Self {
            container,
            push_event: CachePadded::new(Event::new()),
            pop_event: CachePadded::new(Event::new()),
            find_waiter: CachePadded::new(Event::new()),
        }
    }

    pub fn bounded(capacity: NonZeroUsize) -> Self
    where
        C: CreateBounded,
    {
        Self::new(C::new_bounded(capacity))
    }

    pub fn unbounded() -> Self
    where
        C: CreateUnbounded,
    {
        Self::new(C::new_unbounded())
    }
}

impl<C: Container> ZQueue<C> {
    pub fn len(&self) -> usize {
        self.container.len()
    }

    pub fn capacity(&self) -> Option<NonZeroUsize> {
        self.container.capacity()
    }

    pub fn is_empty(&self) -> bool {
        self.container.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.container.is_full()
    }

    pub fn clear(&self) {
        let removed = self.container.clear();
        self.pop_event.notify(removed);
    }

    pub fn try_push(&self, item: C::Item) -> Result<(), C::Item> {
        self.container.push(item)?;

        self.push_event.notify(1);
        self.find_waiter.notify(usize::MAX);
        Ok(())
    }

    pub fn push(&self, mut item: C::Item) {
        let backoff = crossbeam_utils::Backoff::new();
        loop {
            match self.container.push(item) {
                Ok(()) => {
                    self.push_event.notify(1);
                    self.find_waiter.notify(usize::MAX);
                    break;
                }
                Err(v) => item = v,
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            let listener = self.pop_event.listen();
            match self.container.push(item) {
                Ok(()) => {
                    self.push_event.notify(1);
                    self.find_waiter.notify(usize::MAX);
                    break;
                }
                Err(v) => item = v,
            }
            listener.wait();
        }
    }

    pub async fn push_async(&self, mut item: C::Item) {
        let backoff = crossbeam_utils::Backoff::new();
        loop {
            match self.container.push(item) {
                Ok(()) => {
                    self.push_event.notify(1);
                    self.find_waiter.notify(usize::MAX);
                    break;
                }
                Err(v) => item = v,
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            let listener = self.pop_event.listen();
            match self.container.push(item) {
                Ok(()) => {
                    self.push_event.notify(1);
                    self.find_waiter.notify(usize::MAX);
                    break;
                }
                Err(v) => item = v,
            }
            listener.await;
        }
    }

    pub fn try_pop(&self) -> Option<C::Item> {
        let item = self.container.pop();
        if item.is_some() && self.capacity().is_some() {
            self.pop_event.notify(1);
        }
        item
    }

    pub fn pop(&self) -> C::Item {
        let backoff = crossbeam_utils::Backoff::new();
        loop {
            if let Some(item) = self.try_pop() {
                return item;
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            let listener = self.push_event.listen();
            if let Some(item) = self.try_pop() {
                return item;
            }
            listener.wait();
        }
    }

    pub async fn pop_async(&self) -> C::Item {
        let backoff = crossbeam_utils::Backoff::new();
        loop {
            if let Some(item) = self.try_pop() {
                return item;
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            let listener = self.push_event.listen();
            if let Some(item) = self.try_pop() {
                return item;
            }
            listener.await;
        }
    }

    pub fn try_find<F>(&self, find_fn: F) -> Option<C::Item>
    where
        F: FnMut(&C::Item) -> bool,
    {
        let item = self.container.find_pop(find_fn);
        if item.is_some() {
            self.pop_event.notify(1);
        }
        item
    }

    pub fn find<F>(&self, mut find_fn: F) -> C::Item
    where
        F: FnMut(&C::Item) -> bool,
    {
        let backoff = crossbeam_utils::Backoff::new();
        loop {
            if let Some(item) = self.try_find(&mut find_fn) {
                return item;
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            let listener = self.push_event.listen();
            if let Some(item) = self.try_find(&mut find_fn) {
                return item;
            }
            listener.wait();
        }
    }

    pub async fn find_async<F>(&self, mut find_fn: F) -> C::Item
    where
        F: FnMut(&C::Item) -> bool,
    {
        let backoff = crossbeam_utils::Backoff::new();
        loop {
            if let Some(item) = self.try_find(&mut find_fn) {
                return item;
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            let listener = self.push_event.listen();
            if let Some(item) = self.try_find(&mut find_fn) {
                return item;
            }
            listener.await;
        }
    }

    pub fn retain<F>(&self, retain_fn: F)
    where
        F: FnMut(&C::Item) -> bool,
    {
        if self.is_empty() {
            return;
        }

        // Retain into a `Vec<T>` so items are dropped after lock is released.
        let removed = if core::mem::needs_drop::<C::Item>() {
            let mut removed = Vec::new();
            let len = self.len();
            if len <= MAX_SMALL_CAPACITY {
                removed.reserve(len);
            }

            self.container.retain_into(retain_fn, &mut removed);
            removed.len()
        } else {
            self.container.retain(retain_fn)
        };

        self.pop_event.notify(removed);
    }

    pub fn retain_into<F>(&self, retain_fn: F, into: &mut Vec<C::Item>)
    where
        F: FnMut(&C::Item) -> bool,
    {
        if self.is_empty() {
            return;
        }

        let old_len = into.len();
        self.container.retain_into(retain_fn, into);
        let new_len = into.len();

        let removed = old_len - new_len;
        self.pop_event.notify(removed);
    }

    #[cfg(feature = "rand")]
    pub fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R) {
        if self.is_empty() {
            return;
        }

        self.container.rand_shuffle(rng);
    }

    #[cfg(feature = "fastrand")]
    pub fn fastrand_shuffle(&self) {
        if self.is_empty() {
            return;
        }

        self.container.fastrand_shuffle();
    }
}

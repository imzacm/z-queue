use alloc::vec::Vec;
use core::num::NonZeroUsize;
use core::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_utils::CachePadded;
use event_listener::{Event, Listener};

use crate::container::{Container, CreateBounded, CreateUnbounded};

pub const MAX_SMALL_CAPACITY: usize = 1024;

#[derive(Debug)]
pub struct ZQueue<C> {
    container: C,
    has_capacity: bool,
    find_waiters: CachePadded<AtomicUsize>,
    // Notified on push.
    push_event: CachePadded<Event>,
    // Notified on pop.
    pub(crate) pop_event: CachePadded<Event>,
}

impl<C: Container> ZQueue<C> {
    pub fn new(container: C) -> Self {
        let has_capacity = container.capacity().is_some();
        Self {
            container,
            has_capacity,
            find_waiters: CachePadded::new(AtomicUsize::new(0)),
            push_event: CachePadded::new(Event::new()),
            pop_event: CachePadded::new(Event::new()),
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
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.container.len()
    }

    #[inline(always)]
    pub fn capacity(&self) -> Option<NonZeroUsize> {
        self.container.capacity()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.container.is_empty()
    }

    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.container.is_full()
    }

    #[inline(always)]
    pub fn clear(&self) {
        let removed = self.container.clear();
        if self.has_capacity && removed > 0 {
            self.pop_event.notify(removed);
        }
    }

    #[inline(always)]
    pub fn try_push(&self, item: C::Item) -> Result<(), C::Item> {
        self.container.push(item)?;

        let find_waiters = self.find_waiters.load(Ordering::Relaxed);
        self.push_event.notify_additional(find_waiters + 1);
        Ok(())
    }

    pub fn push(&self, mut item: C::Item) {
        if !self.has_capacity {
            let result = self.try_push(item);
            debug_assert!(result.is_ok(), "Unbounded container failed to push");
            return;
        }

        match self.try_push(item) {
            Ok(()) => return,
            Err(v) => item = v,
        }

        let backoff = crossbeam_utils::Backoff::new();
        loop {
            match self.try_push(item) {
                Ok(()) => return,
                Err(v) => item = v,
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            let listener = self.pop_event.listen();
            match self.try_push(item) {
                Ok(()) => return,
                Err(v) => item = v,
            }

            listener.wait();
        }
    }

    pub async fn push_async(&self, mut item: C::Item) {
        if !self.has_capacity {
            let result = self.try_push(item);
            debug_assert!(result.is_ok(), "Unbounded container failed to push");
            return;
        }

        loop {
            match self.try_push(item) {
                Ok(()) => return,
                Err(v) => item = v,
            }

            let listener = self.pop_event.listen();
            match self.try_push(item) {
                Ok(()) => return,
                Err(v) => item = v,
            }

            listener.await;
        }
    }

    #[inline(always)]
    pub fn try_pop(&self) -> Option<C::Item> {
        let item = self.container.pop();
        if item.is_some() && self.has_capacity {
            self.pop_event.notify_additional(1);
        }
        item
    }

    pub fn pop(&self) -> C::Item {
        if let Some(item) = self.try_pop() {
            return item;
        }

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
        loop {
            if let Some(item) = self.try_pop() {
                return item;
            }

            let listener = self.push_event.listen();
            if let Some(item) = self.try_pop() {
                return item;
            }

            listener.await;
        }
    }

    #[inline(always)]
    pub fn try_find<F>(&self, find_fn: F) -> Option<C::Item>
    where
        F: FnMut(&C::Item) -> bool,
    {
        let item = self.container.find_pop(find_fn);
        if item.is_some() && self.has_capacity {
            self.pop_event.notify_additional(1);
        }
        item
    }

    pub fn find<F>(&self, mut find_fn: F) -> C::Item
    where
        F: FnMut(&C::Item) -> bool,
    {
        if let Some(item) = self.try_find(&mut find_fn) {
            return item;
        }

        let backoff = crossbeam_utils::Backoff::new();
        self.find_waiters.fetch_add(1, Ordering::Release);
        loop {
            if let Some(item) = self.try_find(&mut find_fn) {
                self.find_waiters.fetch_sub(1, Ordering::Relaxed);
                return item;
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            let listener = self.push_event.listen();
            if let Some(item) = self.try_find(&mut find_fn) {
                self.find_waiters.fetch_sub(1, Ordering::Release);
                return item;
            }

            listener.wait();
        }
    }

    pub async fn find_async<F>(&self, mut find_fn: F) -> C::Item
    where
        F: FnMut(&C::Item) -> bool,
    {
        self.find_waiters.fetch_add(1, Ordering::Release);
        loop {
            if let Some(item) = self.try_find(&mut find_fn) {
                self.find_waiters.fetch_sub(1, Ordering::Relaxed);
                return item;
            }

            let listener = self.push_event.listen();
            if let Some(item) = self.try_find(&mut find_fn) {
                self.find_waiters.fetch_sub(1, Ordering::Release);
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

        if self.has_capacity && removed > 0 {
            self.pop_event.notify_additional(removed);
        }
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
        let removed = into.len() - old_len;

        if self.has_capacity && removed > 0 {
            self.pop_event.notify_additional(removed);
        }
    }

    pub fn visit<F>(&self, visit_fn: F)
    where
        F: FnMut(&C::Item),
    {
        self.container.visit(visit_fn);
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

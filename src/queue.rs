use alloc::vec::Vec;
use core::num::NonZeroUsize;
use core::sync::atomic::{AtomicU16, Ordering};

use crossbeam_utils::CachePadded;
use z_sync::{Notify, NotifyState, NotifyStateU64};

use crate::container::{Container, CreateBounded, CreateUnbounded};

pub const MAX_SMALL_CAPACITY: usize = 1024;

/// Ensure find_waiters and push_event are on the same cache line.
#[derive(Debug)]
pub(crate) struct PushEvent<PS: NotifyState, OS: NotifyState> {
    push_event: Notify<PS>,
    observe_event: Notify<OS>,
    pub find_waiters: AtomicU16,
}

impl<PS: NotifyState, OS: NotifyState> PushEvent<PS, OS> {
    pub fn new() -> Self {
        Self {
            push_event: Notify::new(),
            observe_event: Notify::new(),
            find_waiters: AtomicU16::new(0),
        }
    }

    #[inline(always)]
    pub fn notify(&self, count: usize) {
        let find_waiters = self.find_waiters.load(Ordering::Relaxed) as usize;
        self.push_event.notify(find_waiters + count);

        self.observe_event.notify(usize::MAX);
    }

    #[inline(always)]
    pub fn listener(&self) -> z_sync::notify::NotifyListener<'_, PS> {
        self.push_event.listener()
    }

    #[inline(always)]
    pub(crate) fn observe(&self) -> z_sync::notify::NotifyListener<'_, OS> {
        self.observe_event.listener()
    }
}

#[derive(Debug)]
pub(crate) struct PopEvent<PS: NotifyState, OS: NotifyState> {
    pop_event: Notify<PS>,
    observe_event: Notify<OS>,
}

impl<PS: NotifyState, OS: NotifyState> PopEvent<PS, OS> {
    pub fn new() -> Self {
        Self { pop_event: Notify::new(), observe_event: Notify::new() }
    }

    #[inline(always)]
    pub fn notify(&self, count: usize) {
        self.pop_event.notify(count);
        self.observe_event.notify(usize::MAX);
    }

    #[inline(always)]
    pub fn listener(&self) -> z_sync::notify::NotifyListener<'_, PS> {
        self.pop_event.listener()
    }

    #[inline(always)]
    pub fn observe(&self) -> z_sync::notify::NotifyListener<'_, OS> {
        self.observe_event.listener()
    }
}

#[derive(Debug)]
pub struct ZQueue<C, PushS = NotifyStateU64, PopS = NotifyStateU64, OS = NotifyStateU64>
where
    PushS: NotifyState,
    PopS: NotifyState,
    OS: NotifyState,
{
    pub(crate) container: C,
    pub(crate) has_capacity: bool,
    // Notified on push.
    pub(crate) push_event: CachePadded<PushEvent<PushS, OS>>,
    // Notified on pop.
    pub(crate) pop_event: CachePadded<PopEvent<PopS, OS>>,
}

impl<C: Container, PushS, PopS, OS> ZQueue<C, PushS, PopS, OS>
where
    PushS: NotifyState,
    PopS: NotifyState,
    OS: NotifyState,
{
    pub fn new(container: C) -> Self {
        let has_capacity = container.capacity().is_some();
        Self {
            container,
            has_capacity,
            push_event: CachePadded::new(PushEvent::new()),
            pop_event: CachePadded::new(PopEvent::new()),
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

impl<C: Container, PushS, PopS, OS> ZQueue<C, PushS, PopS, OS>
where
    PushS: NotifyState,
    PopS: NotifyState,
    OS: NotifyState,
{
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
    pub fn observe_push(&self) -> z_sync::notify::NotifyListener<'_, OS> {
        self.push_event.observe()
    }

    #[inline(always)]
    pub fn observe_pop(&self) -> z_sync::notify::NotifyListener<'_, OS> {
        self.pop_event.observe()
    }

    #[inline(always)]
    pub fn try_push(&self, item: C::Item) -> Result<(), C::Item> {
        self.container.push(item)?;
        self.push_event.notify(1);
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

            let listener = self.pop_event.listener();
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

            let listener = self.pop_event.listener();
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
            self.pop_event.notify(1);
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

            let listener = self.push_event.listener();
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

            let listener = self.push_event.listener();
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
            self.pop_event.notify(1);
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
        self.push_event.find_waiters.fetch_add(1, Ordering::Release);
        loop {
            if let Some(item) = self.try_find(&mut find_fn) {
                self.push_event.find_waiters.fetch_sub(1, Ordering::Relaxed);
                return item;
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            let listener = self.push_event.listener();
            if let Some(item) = self.try_find(&mut find_fn) {
                self.push_event.find_waiters.fetch_sub(1, Ordering::Release);
                return item;
            }

            listener.wait();
        }
    }

    pub async fn find_async<F>(&self, mut find_fn: F) -> C::Item
    where
        F: FnMut(&C::Item) -> bool,
    {
        self.push_event.find_waiters.fetch_add(1, Ordering::Release);
        loop {
            if let Some(item) = self.try_find(&mut find_fn) {
                self.push_event.find_waiters.fetch_sub(1, Ordering::Relaxed);
                return item;
            }

            let listener = self.push_event.listener();
            if let Some(item) = self.try_find(&mut find_fn) {
                self.push_event.find_waiters.fetch_sub(1, Ordering::Release);
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
            self.pop_event.notify(removed);
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
            self.pop_event.notify(removed);
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

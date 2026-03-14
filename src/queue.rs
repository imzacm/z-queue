use alloc::vec::Vec;

use crossbeam_utils::CachePadded;
use event_listener::{Event, Listener};

use crate::container::{Container, ContainerTrait};

pub const MAX_SMALL_CAPACITY: usize = 1024;

#[derive(Debug)]
pub struct ZQueue<T, C: ContainerTrait<T> = Container<T>> {
    container: C,
    // Notified on push.
    push_event: CachePadded<Event>,
    // Notified on pop.
    pub(crate) pop_event: CachePadded<Event>,
    // All waiters are notified on every push.
    find_waiter: CachePadded<Event>,
    _marker: core::marker::PhantomData<T>,
}

impl<T> ZQueue<T, Container<T>> {
    pub fn new<C>(capacity: C) -> Self
    where
        C: Into<Option<usize>>,
    {
        if let Some(capacity) = capacity.into() {
            Self::bounded(capacity)
        } else {
            Self::unbounded()
        }
    }

    pub fn bounded(capacity: usize) -> Self {
        let container = if capacity <= MAX_SMALL_CAPACITY {
            Container::new_vec_dequeue(Some(capacity))
        } else {
            Container::new_segmented_array(Some(capacity))
        };
        Self::with_container(container)
    }

    pub fn unbounded() -> Self {
        Self::with_container(Container::new_segmented_array(None))
    }

    pub fn new_crossbeam<C>(capacity: C) -> Self
    where
        C: Into<Option<usize>>,
    {
        if let Some(capacity) = capacity.into() {
            Self::bounded_crossbeam(capacity)
        } else {
            Self::unbounded_crossbeam()
        }
    }

    pub fn bounded_crossbeam(capacity: usize) -> Self {
        Self::with_container(Container::new_crossbeam_array(capacity))
    }

    pub fn unbounded_crossbeam() -> Self {
        Self::with_container(Container::new_crossbeam_seg())
    }
}

impl<T, C> ZQueue<T, C>
where
    C: ContainerTrait<T>,
{
    pub fn with_container(container: C) -> Self {
        Self {
            container,
            push_event: CachePadded::new(Event::new()),
            pop_event: CachePadded::new(Event::new()),
            find_waiter: CachePadded::new(Event::new()),
            _marker: core::marker::PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.container.len()
    }

    pub fn capacity(&self) -> Option<usize> {
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

    pub fn try_push(&self, item: T) -> Result<(), T> {
        self.container.push(item)?;

        self.push_event.notify(1);
        self.find_waiter.notify(usize::MAX);
        Ok(())
    }

    pub fn push(&self, mut item: T) {
        loop {
            let listener = self.pop_event.listen();

            match self.container.push(item) {
                Ok(()) => {
                    self.push_event.notify(1);
                    self.find_waiter.notify(usize::MAX);
                    break;
                }
                Err(v) => item = v,
            }

            let backoff = crossbeam_utils::Backoff::new();
            while self.is_full() {
                if backoff.is_completed() {
                    listener.wait();
                    break;
                }
                backoff.snooze();
            }
        }
    }

    pub async fn push_async(&self, mut item: T) {
        loop {
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

    pub fn try_pop(&self) -> Option<T> {
        let item = self.container.pop();
        if item.is_some() {
            self.pop_event.notify(1);
        }
        item
    }

    pub fn pop(&self) -> T {
        loop {
            let listener = self.push_event.listen();

            if !self.is_empty() {
                let item = self.container.pop();
                if let Some(item) = item {
                    self.pop_event.notify(1);
                    return item;
                }
            }

            let backoff = crossbeam_utils::Backoff::new();
            while self.is_empty() {
                if backoff.is_completed() {
                    listener.wait();
                    break;
                }
                backoff.snooze();
            }
        }
    }

    pub async fn pop_async(&self) -> T {
        loop {
            let listener = self.push_event.listen();

            if !self.is_empty() {
                let item = self.container.pop();
                if let Some(item) = item {
                    self.pop_event.notify(1);
                    return item;
                }
            }

            listener.await;
        }
    }

    pub fn try_find<F>(&self, find_fn: F) -> Option<T>
    where
        F: FnMut(&T) -> bool,
    {
        if self.is_empty() {
            return None;
        }

        let item = self.container.find_pop(find_fn);
        if item.is_some() {
            self.pop_event.notify(1);
        }
        item
    }

    pub fn find<F>(&self, mut find_fn: F) -> T
    where
        F: FnMut(&T) -> bool,
    {
        loop {
            let listener = self.find_waiter.listen();

            if !self.is_empty() {
                let item = self.container.find_pop(&mut find_fn);
                if let Some(item) = item {
                    self.pop_event.notify(1);
                    return item;
                }
            }

            listener.wait();
        }
    }

    pub async fn find_async<F>(&self, mut find_fn: F) -> T
    where
        F: FnMut(&T) -> bool,
    {
        loop {
            let listener = self.find_waiter.listen();

            if !self.is_empty() {
                let item = self.container.find_pop(&mut find_fn);
                if let Some(item) = item {
                    self.pop_event.notify(1);
                    return item;
                }
            }

            listener.await;
        }
    }

    pub fn retain<F>(&self, retain_fn: F)
    where
        F: FnMut(&T) -> bool,
    {
        if self.is_empty() {
            return;
        }

        // Retain into a `Vec<T>` so items are dropped after lock is released.
        let removed = if core::mem::needs_drop::<T>() {
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

    pub fn retain_into<F>(&self, retain_fn: F, into: &mut Vec<T>)
    where
        F: FnMut(&T) -> bool,
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

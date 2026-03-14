use core::hash::{BuildHasher, Hash};
use core::sync::atomic::{AtomicUsize, Ordering};
use std::num::NonZeroUsize;

use boxcar::Vec;
use crossbeam_utils::CachePadded;
use event_listener::{Event, Listener};
use indexmap::IndexMap;
use parking_lot::RwLock;

use crate::ZQueue;
use crate::queue::MAX_SMALL_CAPACITY;

#[cfg(feature = "std")]
pub type DefaultHashState = std::hash::RandomState;

#[cfg(not(feature = "std"))]
pub type DefaultHashState = foldhash::fast::FixedState;

pub struct ZQueueMap<K, T, S = DefaultHashState> {
    key_index_map: RwLock<IndexMap<K, usize, S>>,
    next_key_index: AtomicUsize,
    queues: Vec<ZQueue<T>>,
    capacity: Option<NonZeroUsize>,
    // Notified on push.
    push_event: CachePadded<Event>,
    // All waiters are notified on every push.
    find_waiter: CachePadded<Event>,
}

impl<K, T, S> ZQueueMap<K, T, S>
where
    K: Clone + Eq + Hash,
    S: BuildHasher + Default,
{
    pub fn new<QC>(key_capacity: usize, queue_capacity: QC) -> Self
    where
        QC: Into<Option<NonZeroUsize>>,
    {
        Self::new_with_hasher(key_capacity, queue_capacity, S::default())
    }

    pub fn bounded(key_capacity: usize, capacity: NonZeroUsize) -> Self {
        Self::new(key_capacity, Some(capacity))
    }

    pub fn unbounded(key_capacity: usize) -> Self {
        Self::new(key_capacity, None)
    }
}

impl<K, T, S> ZQueueMap<K, T, S>
where
    K: Clone + Eq + Hash,
    S: BuildHasher,
{
    pub fn new_with_hasher<QC>(key_capacity: usize, queue_capacity: QC, hasher: S) -> Self
    where
        QC: Into<Option<NonZeroUsize>>,
    {
        Self {
            key_index_map: RwLock::new(IndexMap::with_capacity_and_hasher(key_capacity, hasher)),
            next_key_index: AtomicUsize::new(0),
            queues: Vec::with_capacity(key_capacity),
            capacity: queue_capacity.into(),
            push_event: CachePadded::new(Event::new()),
            find_waiter: CachePadded::new(Event::new()),
        }
    }

    pub fn bounded_with_hasher(key_capacity: usize, capacity: NonZeroUsize, hasher: S) -> Self {
        Self::new_with_hasher(key_capacity, Some(capacity), hasher)
    }

    pub fn unbounded_with_hasher(key_capacity: usize, hasher: S) -> Self {
        Self::new_with_hasher(key_capacity, None, hasher)
    }

    pub fn total_len(&self) -> usize {
        self.queues.iter().map(|(_, q)| q.len()).sum()
    }

    pub fn len(&self, key: &K) -> usize {
        if let Some(&index) = self.key_index_map.read().get(key) {
            return self.queues[index].len();
        }
        0
    }

    pub fn capacity(&self) -> Option<NonZeroUsize> {
        self.capacity
    }

    pub fn is_empty(&self, key: &K) -> bool {
        if let Some(&index) = self.key_index_map.read().get(key) {
            return self.queues[index].is_empty();
        }
        true
    }

    pub fn is_full(&self, key: &K) -> bool {
        if let Some(&index) = self.key_index_map.read().get(key) {
            return self.queues[index].is_full();
        }
        false
    }

    pub fn clear(&self) {
        for (_, queue) in self.queues.iter() {
            queue.clear();
        }
    }

    pub fn try_push(&self, key: &K, item: T) -> Result<(), T> {
        if let Some(&index) = self.key_index_map.read().get(key) {
            let queue = &self.queues[index];
            let result = queue.try_push(item);
            if result.is_ok() {
                self.push_event.notify(1);
                self.find_waiter.notify(usize::MAX);
            }
            return result;
        }

        let mut lock = self.key_index_map.write();
        if let Some(&index) = lock.get(key) {
            drop(lock);
            let queue = &self.queues[index];
            let result = queue.try_push(item);
            if result.is_ok() {
                self.push_event.notify(1);
                self.find_waiter.notify(usize::MAX);
            }
            return result;
        }

        let queue = ZQueue::new(self.capacity.map(NonZeroUsize::get));
        queue.push(item);
        let index = self.queues.push(queue);
        lock.insert(key.clone(), index);
        drop(lock);
        self.push_event.notify(1);
        self.find_waiter.notify(usize::MAX);
        Ok(())
    }

    pub fn push(&self, key: &K, item: T) {
        if let Some(&index) = self.key_index_map.read().get(key) {
            let queue = &self.queues[index];
            queue.push(item);
            self.push_event.notify(1);
            self.find_waiter.notify(usize::MAX);
            return;
        }

        let mut lock = self.key_index_map.write();
        if let Some(&index) = lock.get(key) {
            drop(lock);
            let queue = &self.queues[index];
            queue.push(item);
            self.push_event.notify(1);
            self.find_waiter.notify(usize::MAX);
            return;
        }

        let queue = ZQueue::new(self.capacity.map(NonZeroUsize::get));
        queue.push(item);
        let index = self.queues.push(queue);
        lock.insert(key.clone(), index);
        drop(lock);
        self.push_event.notify(1);
        self.find_waiter.notify(usize::MAX);
    }

    #[expect(clippy::await_holding_lock, reason = "False positive, lock is dropped before await")]
    pub async fn push_async(&self, key: &K, mut item: T) {
        if let Some(&index) = self.key_index_map.read().get(key) {
            let queue = &self.queues[index];

            loop {
                let listener = queue.pop_event.listen();

                match queue.try_push(item) {
                    Ok(()) => {
                        self.push_event.notify(1);
                        self.find_waiter.notify(usize::MAX);
                        return;
                    }
                    Err(v) => item = v,
                }

                listener.await;
            }
        }

        let mut lock = self.key_index_map.write();
        if let Some(&index) = lock.get(key) {
            drop(lock);
            let queue = &self.queues[index];

            loop {
                let listener = queue.pop_event.listen();

                match queue.try_push(item) {
                    Ok(()) => {
                        self.push_event.notify(1);
                        self.find_waiter.notify(usize::MAX);
                        return;
                    }
                    Err(v) => item = v,
                }

                listener.await;
            }
        }

        let queue = ZQueue::new(self.capacity.map(NonZeroUsize::get));
        queue.push(item);
        let index = self.queues.push(queue);
        lock.insert(key.clone(), index);
        drop(lock);
        self.push_event.notify(1);
        self.find_waiter.notify(usize::MAX);
    }

    pub fn try_pop<KF>(&self, mut key_fn: KF) -> Option<(K, T)>
    where
        KF: FnMut(&K) -> bool,
    {
        let lock = self.key_index_map.read();
        if lock.is_empty() {
            return None;
        }

        let initial_key_index = self.next_key_index.fetch_add(1, Ordering::Relaxed);

        for index in 0..lock.len() {
            let key_index = (initial_key_index + index) % lock.len();
            let (key, index) = lock.get_index(key_index).unwrap();
            if key_fn(key)
                && let Some(item) = self.queues[*index].try_pop()
            {
                return Some((key.clone(), item));
            }
        }

        None
    }

    pub fn pop<KF>(&self, mut key_fn: KF) -> (K, T)
    where
        KF: FnMut(&K) -> bool,
    {
        loop {
            let listener = self.push_event.listen();

            {
                let lock = self.key_index_map.read();

                if lock.is_empty() {
                    drop(lock);
                    listener.wait();
                    continue;
                }

                let initial_key_index = self.next_key_index.fetch_add(1, Ordering::Relaxed);

                for index in 0..lock.len() {
                    let key_index = (initial_key_index + index) % lock.len();
                    let (key, index) = lock.get_index(key_index).unwrap();
                    if key_fn(key)
                        && let Some(item) = self.queues[*index].try_pop()
                    {
                        return (key.clone(), item);
                    }
                }
            }

            listener.wait();
        }
    }

    #[expect(clippy::await_holding_lock, reason = "False positive, lock is dropped before await")]
    pub async fn pop_async<KF>(&self, mut key_fn: KF) -> (K, T)
    where
        KF: FnMut(&K) -> bool,
    {
        loop {
            let listener = self.push_event.listen();

            {
                let lock = self.key_index_map.read();

                if lock.is_empty() {
                    drop(lock);
                    listener.await;
                    continue;
                }

                let initial_key_index = self.next_key_index.fetch_add(1, Ordering::Relaxed);

                for index in 0..lock.len() {
                    let key_index = (initial_key_index + index) % lock.len();
                    let (key, index) = lock.get_index(key_index).unwrap();
                    if key_fn(key)
                        && let Some(item) = self.queues[*index].try_pop()
                    {
                        return (key.clone(), item);
                    }
                }
            }

            listener.await;
        }
    }

    pub fn try_find<KF, F>(&self, mut key_fn: KF, mut find_fn: F) -> Option<(K, T)>
    where
        KF: FnMut(&K) -> bool,
        F: FnMut(&T) -> bool,
    {
        let lock = self.key_index_map.read();

        if lock.is_empty() {
            return None;
        }

        let initial_key_index = self.next_key_index.fetch_add(1, Ordering::Relaxed);

        for index in 0..lock.len() {
            let key_index = (initial_key_index + index) % lock.len();
            let (key, index) = lock.get_index(key_index).unwrap();
            if key_fn(key)
                && let Some(item) = self.queues[*index].try_find(&mut find_fn)
            {
                return Some((key.clone(), item));
            }
        }

        None
    }

    pub fn find<KF, F>(&self, mut key_fn: KF, mut find_fn: F) -> (K, T)
    where
        KF: FnMut(&K) -> bool,
        F: FnMut(&T) -> bool,
    {
        loop {
            let listener = self.find_waiter.listen();

            {
                let lock = self.key_index_map.read();

                if lock.is_empty() {
                    drop(lock);
                    listener.wait();
                    continue;
                }

                let initial_key_index = self.next_key_index.fetch_add(1, Ordering::Relaxed);

                for index in 0..lock.len() {
                    let key_index = (initial_key_index + index) % lock.len();
                    let (key, index) = lock.get_index(key_index).unwrap();
                    if key_fn(key)
                        && let Some(item) = self.queues[*index].try_find(&mut find_fn)
                    {
                        return (key.clone(), item);
                    }
                }
            }

            listener.wait();
        }
    }

    #[expect(clippy::await_holding_lock, reason = "False positive, lock is dropped before await")]
    pub async fn find_async<KF, F>(&self, mut key_fn: KF, mut find_fn: F) -> (K, T)
    where
        KF: FnMut(&K) -> bool,
        F: FnMut(&T) -> bool,
    {
        loop {
            let listener = self.find_waiter.listen();

            {
                let lock = self.key_index_map.read();

                if lock.is_empty() {
                    drop(lock);
                    listener.await;
                    continue;
                }

                let initial_key_index = self.next_key_index.fetch_add(1, Ordering::Relaxed);

                for index in 0..lock.len() {
                    let key_index = (initial_key_index + index) % lock.len();
                    let (key, index) = lock.get_index(key_index).unwrap();
                    if key_fn(key)
                        && let Some(item) = self.queues[*index].try_find(&mut find_fn)
                    {
                        return (key.clone(), item);
                    }
                }
            }

            listener.await;
        }
    }

    pub fn retain<KF, F>(&self, mut key_fn: KF, mut retain_fn: F)
    where
        KF: FnMut(&K) -> bool,
        F: FnMut(&T) -> bool,
    {
        // Retain into a `Vec<T>` if `T` needs drop, so items are dropped after lock is released.
        let mut removed = alloc::vec::Vec::new();

        let lock = self.key_index_map.read();
        for (key, index) in lock.iter() {
            if key_fn(key) {
                let queue = &self.queues[*index];

                if core::mem::needs_drop::<T>() {
                    let len = queue.len();
                    if len <= MAX_SMALL_CAPACITY {
                        removed.reserve(len);
                    }
                    queue.retain_into(&mut retain_fn, &mut removed);
                } else {
                    queue.retain(&mut retain_fn);
                }
            }
        }
        drop(lock);
    }

    pub fn retain_into<KF, F>(
        &self,
        mut key_fn: KF,
        mut retain_fn: F,
        into: &mut alloc::vec::Vec<T>,
    ) where
        KF: FnMut(&K) -> bool,
        F: FnMut(&T) -> bool,
    {
        let lock = self.key_index_map.read();
        for (key, index) in lock.iter() {
            if key_fn(key) {
                let queue = &self.queues[*index];

                queue.retain_into(&mut retain_fn, into);
            }
        }
        drop(lock);
    }

    #[cfg(feature = "rand")]
    pub fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R) {
        for (_, queue) in self.queues.iter() {
            queue.rand_shuffle(rng);
        }
    }

    #[cfg(feature = "fastrand")]
    pub fn fastrand_shuffle(&self) {
        for (_, queue) in self.queues.iter() {
            queue.fastrand_shuffle();
        }
    }
}

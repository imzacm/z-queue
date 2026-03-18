use core::hash::{BuildHasher, Hash};
use core::num::NonZeroUsize;
use core::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_queue::SegQueue;
use crossbeam_utils::CachePadded;
use event_listener::{Event, Listener};

use crate::ZQueue;
use crate::container::{Container, CreateBounded, CreateUnbounded};

#[cfg(feature = "std")]
pub type DefaultHashState = std::hash::RandomState;

#[cfg(not(feature = "std"))]
pub type DefaultHashState = foldhash::fast::FixedState;

#[derive(Debug)]
enum CreateContainer<C> {
    Unbounded(fn() -> C),
    Bounded(NonZeroUsize, fn(NonZeroUsize) -> C),
}

pub struct ZQueueMap<K, C, S: BuildHasher = DefaultHashState> {
    queues: scc::HashMap<K, ZQueue<C>, S>,
    ready_keys: SegQueue<K>,
    find_waiters: CachePadded<AtomicUsize>,
    create_container: CreateContainer<C>,
    // Notified on push.
    push_event: CachePadded<Event>,
}

impl<K, C, S> ZQueueMap<K, C, S>
where
    K: Clone + Eq + Hash,
    C: Container,
    S: BuildHasher + Default,
{
    pub fn bounded(key_capacity: usize, queue_capacity: NonZeroUsize) -> Self
    where
        C: CreateBounded,
    {
        Self::bounded_with_hasher(key_capacity, queue_capacity, S::default())
    }

    pub fn unbounded(key_capacity: usize) -> Self
    where
        C: CreateUnbounded,
    {
        Self::unbounded_with_hasher(key_capacity, S::default())
    }
}

impl<K, C, S> ZQueueMap<K, C, S>
where
    K: Clone + Eq + Hash,
    C: Container,
    S: BuildHasher,
{
    pub fn bounded_with_hasher(key_capacity: usize, queue_capacity: NonZeroUsize, hasher: S) -> Self
    where
        C: CreateBounded,
    {
        Self {
            queues: scc::HashMap::with_capacity_and_hasher(key_capacity, hasher),
            ready_keys: SegQueue::new(),
            find_waiters: CachePadded::new(AtomicUsize::new(0)),
            create_container: CreateContainer::Bounded(queue_capacity, C::new_bounded),
            push_event: CachePadded::new(Event::new()),
        }
    }

    pub fn unbounded_with_hasher(key_capacity: usize, hasher: S) -> Self
    where
        C: CreateUnbounded,
    {
        Self {
            queues: scc::HashMap::with_hasher(hasher),
            ready_keys: SegQueue::new(),
            find_waiters: CachePadded::new(AtomicUsize::new(0)),
            create_container: CreateContainer::Unbounded(C::new_unbounded),
            push_event: CachePadded::new(Event::new()),
        }
    }

    fn create_queue(&self) -> ZQueue<C> {
        match &self.create_container {
            CreateContainer::Unbounded(f) => ZQueue::new(f()),
            CreateContainer::Bounded(cap, f) => ZQueue::new(f(*cap)),
        }
    }

    pub fn total_len(&self) -> usize {
        let mut len = 0;
        self.queues.iter_sync(|_, queue| {
            len += queue.len();
            true
        });
        len
    }

    pub fn len(&self, key: &K) -> usize {
        self.queues.get_sync(key).map_or(0, |queue| queue.len())
    }

    pub fn capacity(&self) -> Option<NonZeroUsize> {
        match &self.create_container {
            CreateContainer::Unbounded(_) => None,
            CreateContainer::Bounded(cap, _) => Some(*cap),
        }
    }

    pub fn is_empty(&self, key: &K) -> bool {
        self.queues.get_sync(key).is_none_or(|queue| queue.is_empty())
    }

    pub fn is_full(&self, key: &K) -> bool {
        if matches!(self.create_container, CreateContainer::Unbounded(_)) {
            return false;
        }

        self.queues.get_sync(key).is_some_and(|queue| queue.is_full())
    }

    pub fn clear(&self) {
        self.queues.clear_sync();
    }

    pub fn try_push(&self, key: K, item: C::Item) -> Result<(), C::Item> {
        let queue = self.queues.entry_sync(key.clone()).or_insert_with(|| self.create_queue());

        queue.try_push(item)?;

        self.ready_keys.push(key);
        let find_waiters = self.find_waiters.load(Ordering::Relaxed);
        self.push_event.notify_additional(find_waiters + 1);
        Ok(())
    }

    pub fn push(&self, key: K, item: C::Item) {
        let queue = self.queues.entry_sync(key.clone()).or_insert_with(|| self.create_queue());
        queue.push(item);
        self.ready_keys.push(key);
        let find_waiters = self.find_waiters.load(Ordering::Relaxed);
        self.push_event.notify_additional(find_waiters + 1);
    }

    pub async fn push_async(&self, key: K, item: C::Item) {
        let queue = self
            .queues
            .entry_async(key.clone())
            .await
            .or_insert_with(|| self.create_queue());
        queue.push_async(item).await;
        self.ready_keys.push(key);
        let find_waiters = self.find_waiters.load(Ordering::Relaxed);
        self.push_event.notify_additional(find_waiters + 1);
    }

    pub fn try_pop<KF>(&self, mut key_fn: KF) -> Option<(K, C::Item)>
    where
        KF: FnMut(&K) -> bool,
    {
        for _ in 0..self.ready_keys.len() {
            let Some(key) = self.ready_keys.pop() else { break };
            if !key_fn(&key) {
                self.ready_keys.push(key);
                continue;
            }

            let Some(queue) = self.queues.get_sync(&key) else { continue };
            if let Some(item) = queue.try_pop() {
                return Some((key, item));
            }
        }
        None
    }

    pub fn pop<KF>(&self, mut key_fn: KF) -> (K, C::Item)
    where
        KF: FnMut(&K) -> bool,
    {
        loop {
            for _ in 0..self.ready_keys.len() {
                let Some(key) = self.ready_keys.pop() else { break };
                if !key_fn(&key) {
                    self.ready_keys.push(key);
                    continue;
                }

                let Some(queue) = self.queues.get_sync(&key) else { continue };
                if let Some(item) = queue.try_pop() {
                    return (key, item);
                }
            }

            let listener = self.push_event.listen();

            for _ in 0..self.ready_keys.len() {
                let Some(key) = self.ready_keys.pop() else { break };
                if !key_fn(&key) {
                    self.ready_keys.push(key);
                    continue;
                }

                let Some(queue) = self.queues.get_sync(&key) else { continue };
                if let Some(item) = queue.try_pop() {
                    return (key, item);
                }
            }

            listener.wait();
        }
    }

    pub async fn pop_async<KF>(&self, mut key_fn: KF) -> (K, C::Item)
    where
        KF: FnMut(&K) -> bool,
    {
        loop {
            for _ in 0..self.ready_keys.len() {
                let Some(key) = self.ready_keys.pop() else { break };
                if !key_fn(&key) {
                    self.ready_keys.push(key);
                    continue;
                }

                let Some(queue) = self.queues.get_async(&key).await else { continue };
                if let Some(item) = queue.try_pop() {
                    return (key, item);
                }
            }

            let listener = self.push_event.listen();

            for _ in 0..self.ready_keys.len() {
                let Some(key) = self.ready_keys.pop() else { break };
                if !key_fn(&key) {
                    self.ready_keys.push(key);
                    continue;
                }

                let Some(queue) = self.queues.get_async(&key).await else { continue };
                if let Some(item) = queue.try_pop() {
                    return (key, item);
                }
            }

            listener.await;
        }
    }

    pub fn try_find<KF, F>(&self, mut key_fn: KF, mut find_fn: F) -> Option<(K, C::Item)>
    where
        KF: FnMut(&K) -> bool,
        F: FnMut(&C::Item) -> bool,
    {
        for _ in 0..self.ready_keys.len() {
            let Some(key) = self.ready_keys.pop() else { break };
            if !key_fn(&key) {
                self.ready_keys.push(key);
                continue;
            }

            let Some(queue) = self.queues.get_sync(&key) else { continue };
            if let Some(item) = queue.try_find(&mut find_fn) {
                return Some((key, item));
            }
        }

        None
    }

    pub fn find<KF, F>(&self, mut key_fn: KF, mut find_fn: F) -> (K, C::Item)
    where
        KF: FnMut(&K) -> bool,
        F: FnMut(&C::Item) -> bool,
    {
        self.find_waiters.fetch_add(1, Ordering::Release);
        loop {
            for _ in 0..self.ready_keys.len() {
                let Some(key) = self.ready_keys.pop() else { break };
                if !key_fn(&key) {
                    self.ready_keys.push(key);
                    continue;
                }

                let Some(queue) = self.queues.get_sync(&key) else { continue };
                if let Some(item) = queue.try_find(&mut find_fn) {
                    self.find_waiters.fetch_sub(1, Ordering::Release);
                    return (key, item);
                }
            }

            let listener = self.push_event.listen();

            for _ in 0..self.ready_keys.len() {
                let Some(key) = self.ready_keys.pop() else { break };
                if !key_fn(&key) {
                    self.ready_keys.push(key);
                    continue;
                }

                let Some(queue) = self.queues.get_sync(&key) else { continue };
                if let Some(item) = queue.try_find(&mut find_fn) {
                    self.find_waiters.fetch_sub(1, Ordering::Release);
                    return (key, item);
                }
            }

            listener.wait();
        }
    }

    pub async fn find_async<KF, F>(&self, mut key_fn: KF, mut find_fn: F) -> (K, C::Item)
    where
        KF: FnMut(&K) -> bool,
        F: FnMut(&C::Item) -> bool,
    {
        self.find_waiters.fetch_add(1, Ordering::Release);
        loop {
            for _ in 0..self.ready_keys.len() {
                let Some(key) = self.ready_keys.pop() else { break };
                if !key_fn(&key) {
                    self.ready_keys.push(key);
                    continue;
                }

                let Some(queue) = self.queues.get_async(&key).await else { continue };
                if let Some(item) = queue.try_find(&mut find_fn) {
                    self.find_waiters.fetch_sub(1, Ordering::Release);
                    return (key, item);
                }
            }

            let listener = self.push_event.listen();

            for _ in 0..self.ready_keys.len() {
                let Some(key) = self.ready_keys.pop() else { break };
                if !key_fn(&key) {
                    self.ready_keys.push(key);
                    continue;
                }

                let Some(queue) = self.queues.get_async(&key).await else { continue };
                if let Some(item) = queue.try_find(&mut find_fn) {
                    self.find_waiters.fetch_sub(1, Ordering::Release);
                    return (key, item);
                }
            }

            listener.await;
        }
    }

    pub fn retain<KF, F>(&self, mut key_fn: KF, mut retain_fn: F)
    where
        KF: FnMut(&K) -> bool,
        F: FnMut(&C::Item) -> bool,
    {
        self.queues.iter_sync(|key, queue| {
            if key_fn(key) {
                queue.retain(&mut retain_fn);
            }
            true
        });
    }

    pub fn retain_into<KF, F>(
        &self,
        mut key_fn: KF,
        mut retain_fn: F,
        into: &mut alloc::vec::Vec<C::Item>,
    ) where
        KF: FnMut(&K) -> bool,
        F: FnMut(&C::Item) -> bool,
    {
        self.queues.iter_sync(|key, queue| {
            if key_fn(key) {
                queue.retain_into(&mut retain_fn, into);
            }
            true
        });
    }

    pub fn visit<F>(&self, mut visit_fn: F)
    where
        F: FnMut(&K, &C::Item),
    {
        self.queues.iter_sync(|key, queue| {
            queue.visit(|item| visit_fn(key, item));
            true
        });
    }

    #[cfg(feature = "rand")]
    pub fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R) {
        self.queues.iter_sync(|_, queue| {
            queue.rand_shuffle(rng);
            true
        });
    }

    #[cfg(feature = "fastrand")]
    pub fn fastrand_shuffle(&self) {
        self.queues.iter_sync(|_, queue| {
            queue.fastrand_shuffle();
            true
        });
    }
}

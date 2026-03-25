#[cfg(not(feature = "triomphe"))]
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::hash::{BuildHasher, Hash};
use core::num::NonZeroUsize;
use core::sync::atomic::{AtomicUsize, Ordering};

use arc_swap::ArcSwapAny;
use crossbeam_utils::CachePadded;
#[cfg(feature = "triomphe")]
use triomphe::Arc;

use crate::ZQueue;
use crate::container::{Container, CreateBounded, CreateUnbounded};
use crate::notify::Notify;

// Use triomphe if enabled.
type ArcSwap<T> = ArcSwapAny<Arc<T>>;

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
    queues: scc::HashMap<K, Arc<ZQueue<C>>, S>,
    keys: ArcSwap<Vec<K>>,
    find_waiters: CachePadded<AtomicUsize>,
    create_container: CreateContainer<C>,
    // Notified on push.
    push_event: Notify,
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
            keys: ArcSwap::new(Arc::new(Vec::with_capacity(key_capacity))),
            find_waiters: CachePadded::new(AtomicUsize::new(0)),
            create_container: CreateContainer::Bounded(queue_capacity, C::new_bounded),
            push_event: Notify::new(),
        }
    }

    pub fn unbounded_with_hasher(key_capacity: usize, hasher: S) -> Self
    where
        C: CreateUnbounded,
    {
        Self {
            queues: scc::HashMap::with_capacity_and_hasher(key_capacity, hasher),
            keys: ArcSwap::new(Arc::new(Vec::with_capacity(key_capacity))),
            find_waiters: CachePadded::new(AtomicUsize::new(0)),
            create_container: CreateContainer::Unbounded(C::new_unbounded),
            push_event: Notify::new(),
        }
    }

    #[inline(always)]
    fn create_queue(&self) -> ZQueue<C> {
        match &self.create_container {
            CreateContainer::Unbounded(f) => ZQueue::new(f()),
            CreateContainer::Bounded(cap, f) => ZQueue::new(f(*cap)),
        }
    }

    fn push_key(&self, key: &K) {
        self.keys.rcu(|keys| {
            let mut keys = Vec::clone(keys);
            keys.push(key.clone());
            keys
        });
    }

    fn rotate_keys(&self) {
        self.keys.rcu(|keys| {
            let mut keys = Vec::clone(keys);
            if !keys.is_empty() {
                keys.rotate_left(1);
            }
            keys
        });
    }

    fn ensure_queue_sync(&self, key: &K) -> Arc<ZQueue<C>> {
        let mut is_new = false;
        let queue = self.queues.entry_sync(key.clone()).or_insert_with(|| {
            is_new = true;
            Arc::new(self.create_queue())
        });

        if is_new {
            self.push_key(key);
        }

        queue.clone()
    }

    async fn ensure_queue_async(&self, key: &K) -> Arc<ZQueue<C>> {
        let mut is_new = false;
        let queue = self.queues.entry_async(key.clone()).await.or_insert_with(|| {
            is_new = true;
            Arc::new(self.create_queue())
        });

        if is_new {
            self.push_key(key);
        }

        queue.clone()
    }

    #[inline(always)]
    pub fn total_len(&self) -> usize {
        let mut len = 0;
        let keys = self.keys.load();
        for key in keys.iter() {
            len += self.len(key);
        }
        len
    }

    #[inline(always)]
    pub async fn total_len_async(&self) -> usize {
        let mut len = 0;
        let keys = self.keys.load();
        for key in keys.iter() {
            len += self.len_async(key).await;
        }
        len
    }

    #[inline(always)]
    pub fn len(&self, key: &K) -> usize {
        self.queues.get_sync(key).map_or(0, |queue| queue.len())
    }

    #[inline(always)]
    pub async fn len_async(&self, key: &K) -> usize {
        self.queues.get_async(key).await.map_or(0, |queue| queue.len())
    }

    #[inline(always)]
    pub fn capacity(&self) -> Option<NonZeroUsize> {
        match &self.create_container {
            CreateContainer::Unbounded(_) => None,
            CreateContainer::Bounded(cap, _) => Some(*cap),
        }
    }

    #[inline(always)]
    pub fn is_empty(&self, key: &K) -> bool {
        self.queues.get_sync(key).is_none_or(|queue| queue.is_empty())
    }

    #[inline(always)]
    pub async fn is_empty_async(&self, key: &K) -> bool {
        self.queues.get_async(key).await.is_none_or(|queue| queue.is_empty())
    }

    #[inline(always)]
    pub fn is_full(&self, key: &K) -> bool {
        if matches!(self.create_container, CreateContainer::Unbounded(_)) {
            return false;
        }

        self.queues.get_sync(key).is_some_and(|queue| queue.is_full())
    }

    #[inline(always)]
    pub async fn is_full_async(&self, key: &K) -> bool {
        if matches!(self.create_container, CreateContainer::Unbounded(_)) {
            return false;
        }

        self.queues.get_async(key).await.is_some_and(|queue| queue.is_full())
    }

    #[inline(always)]
    pub fn clear(&self) {
        self.queues.clear_sync();
    }

    #[inline(always)]
    pub async fn clear_async(&self) {
        self.queues.clear_async().await;
    }

    #[inline(always)]
    pub fn try_push(&self, key: K, item: C::Item) -> Result<(), C::Item> {
        let queue = self.ensure_queue_sync(&key);

        queue.try_push(item)?;

        let find_waiters = self.find_waiters.load(Ordering::Relaxed);
        self.push_event.notify(find_waiters + 1);
        Ok(())
    }

    #[inline(always)]
    pub async fn try_push_async(&self, key: K, item: C::Item) -> Result<(), C::Item> {
        let queue = self.ensure_queue_async(&key).await;

        queue.try_push(item)?;

        let find_waiters = self.find_waiters.load(Ordering::Relaxed);
        self.push_event.notify(find_waiters + 1);
        Ok(())
    }

    #[inline(always)]
    pub fn push(&self, key: K, item: C::Item) {
        let queue = self.ensure_queue_sync(&key);
        queue.push(item);
        let find_waiters = self.find_waiters.load(Ordering::Relaxed);
        self.push_event.notify(find_waiters + 1);
    }

    #[inline(always)]
    pub async fn push_async(&self, key: K, item: C::Item) {
        let queue = self.ensure_queue_async(&key).await;
        queue.push_async(item).await;
        let find_waiters = self.find_waiters.load(Ordering::Relaxed);
        self.push_event.notify(find_waiters + 1);
    }

    pub fn try_pop<KF>(&self, mut key_fn: KF) -> Option<(K, C::Item)>
    where
        KF: FnMut(&K) -> bool,
    {
        let keys = self.keys.load();
        for key in keys.iter() {
            if !key_fn(key) {
                continue;
            }

            let Some(queue) = self.queues.get_sync(key) else { continue };
            if let Some(item) = queue.try_pop() {
                self.rotate_keys();
                return Some((key.clone(), item));
            }
        }

        None
    }

    pub async fn try_pop_async<KF>(&self, mut key_fn: KF) -> Option<(K, C::Item)>
    where
        KF: FnMut(&K) -> bool,
    {
        let keys = self.keys.load();
        for key in keys.iter() {
            if !key_fn(key) {
                continue;
            }

            let Some(queue) = self.queues.get_async(key).await else { continue };
            if let Some(item) = queue.try_pop() {
                self.rotate_keys();
                return Some((key.clone(), item));
            }
        }

        None
    }

    pub fn pop<KF>(&self, mut key_fn: KF) -> (K, C::Item)
    where
        KF: FnMut(&K) -> bool,
    {
        if let Some(item) = self.try_pop(&mut key_fn) {
            return item;
        }

        let backoff = crossbeam_utils::Backoff::new();
        loop {
            if let Some(item) = self.try_pop(&mut key_fn) {
                return item;
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            let listener = self.push_event.listener();
            if let Some(item) = self.try_pop(&mut key_fn) {
                return item;
            }

            listener.wait();
        }
    }

    pub async fn pop_async<KF>(&self, mut key_fn: KF) -> (K, C::Item)
    where
        KF: FnMut(&K) -> bool,
    {
        loop {
            if let Some(item) = self.try_pop_async(&mut key_fn).await {
                return item;
            }

            let listener = self.push_event.listener();
            if let Some(item) = self.try_pop_async(&mut key_fn).await {
                return item;
            }

            listener.await;
        }
    }

    pub fn try_find<KF, F>(&self, mut key_fn: KF, mut find_fn: F) -> Option<(K, C::Item)>
    where
        KF: FnMut(&K) -> bool,
        F: FnMut(&C::Item) -> bool,
    {
        let keys = self.keys.load();
        for key in keys.iter() {
            if !key_fn(key) {
                continue;
            }

            let Some(queue) = self.queues.get_sync(key) else { continue };
            if let Some(item) = queue.try_find(&mut find_fn) {
                self.rotate_keys();
                return Some((key.clone(), item));
            }
        }

        None
    }

    pub async fn try_find_async<KF, F>(
        &self,
        mut key_fn: KF,
        mut find_fn: F,
    ) -> Option<(K, C::Item)>
    where
        KF: FnMut(&K) -> bool,
        F: FnMut(&C::Item) -> bool,
    {
        let keys = self.keys.load();
        for key in keys.iter() {
            if !key_fn(key) {
                continue;
            }

            let Some(queue) = self.queues.get_async(key).await else { continue };
            if let Some(item) = queue.try_find(&mut find_fn) {
                self.rotate_keys();
                return Some((key.clone(), item));
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
        let _guard = FindWaiterGuard { count: &self.find_waiters };

        if let Some(item) = self.try_find(&mut key_fn, &mut find_fn) {
            return item;
        }

        let backoff = crossbeam_utils::Backoff::new();
        loop {
            if let Some(item) = self.try_find(&mut key_fn, &mut find_fn) {
                return item;
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            let listener = self.push_event.listener();
            if let Some(item) = self.try_find(&mut key_fn, &mut find_fn) {
                return item;
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
        let _guard = FindWaiterGuard { count: &self.find_waiters };

        loop {
            if let Some(item) = self.try_find_async(&mut key_fn, &mut find_fn).await {
                return item;
            }

            let listener = self.push_event.listener();
            if let Some(item) = self.try_find_async(&mut key_fn, &mut find_fn).await {
                return item;
            }

            listener.await;
        }
    }

    pub fn retain<KF, F>(&self, mut key_fn: KF, mut retain_fn: F)
    where
        KF: FnMut(&K) -> bool,
        F: FnMut(&C::Item) -> bool,
    {
        let keys = self.keys.load();
        for key in keys.iter() {
            if key_fn(key)
                && let Some(queue) = self.queues.get_sync(key).map(|q| q.clone())
            {
                queue.retain(&mut retain_fn);
            }
        }
    }

    pub async fn retain_async<KF, F>(&self, mut key_fn: KF, mut retain_fn: F)
    where
        KF: FnMut(&K) -> bool,
        F: FnMut(&C::Item) -> bool,
    {
        let keys = self.keys.load();
        for key in keys.iter() {
            if key_fn(key)
                && let Some(queue) = self.queues.get_async(key).await.map(|q| q.clone())
            {
                queue.retain(&mut retain_fn);
            }
        }
    }

    pub fn retain_into<KF, F>(&self, mut key_fn: KF, mut retain_fn: F, into: &mut Vec<C::Item>)
    where
        KF: FnMut(&K) -> bool,
        F: FnMut(&C::Item) -> bool,
    {
        let keys = self.keys.load();
        for key in keys.iter() {
            if key_fn(key)
                && let Some(queue) = self.queues.get_sync(key).map(|q| q.clone())
            {
                queue.retain_into(&mut retain_fn, into);
            }
        }
    }

    pub async fn retain_into_async<KF, F>(
        &self,
        mut key_fn: KF,
        mut retain_fn: F,
        into: &mut Vec<C::Item>,
    ) where
        KF: FnMut(&K) -> bool,
        F: FnMut(&C::Item) -> bool,
    {
        let keys = self.keys.load();
        for key in keys.iter() {
            if key_fn(key)
                && let Some(queue) = self.queues.get_async(key).await.map(|q| q.clone())
            {
                queue.retain_into(&mut retain_fn, into);
            }
        }
    }

    pub fn visit<F>(&self, mut visit_fn: F)
    where
        F: FnMut(&K, &C::Item),
    {
        let keys = self.keys.load();
        for key in keys.iter() {
            let Some(queue) = self.queues.get_sync(key).map(|q| q.clone()) else { continue };
            queue.visit(|item| visit_fn(key, item));
        }
    }

    pub async fn visit_async<F>(&self, mut visit_fn: F)
    where
        F: FnMut(&K, &C::Item),
    {
        let keys = self.keys.load();
        for key in keys.iter() {
            let Some(queue) = self.queues.get_async(key).await.map(|q| q.clone()) else { continue };
            queue.visit(|item| visit_fn(key, item));
        }
    }

    #[cfg(feature = "rand")]
    pub fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R) {
        let keys = self.keys.load();
        for key in keys.iter() {
            let Some(queue) = self.queues.get_sync(key).map(|q| q.clone()) else { continue };
            queue.rand_shuffle(rng);
        }
    }

    #[cfg(feature = "rand")]
    pub async fn rand_shuffle_async<R: rand::Rng>(&self, rng: &mut R) {
        let keys = self.keys.load();
        for key in keys.iter() {
            let Some(queue) = self.queues.get_async(key).await.map(|q| q.clone()) else { continue };
            queue.rand_shuffle(rng);
        }
    }

    #[cfg(feature = "fastrand")]
    pub fn fastrand_shuffle(&self) {
        let keys = self.keys.load();
        for key in keys.iter() {
            let Some(queue) = self.queues.get_sync(key).map(|q| q.clone()) else { continue };
            queue.fastrand_shuffle();
        }
    }

    #[cfg(feature = "fastrand")]
    pub async fn fastrand_shuffle_async(&self) {
        let keys = self.keys.load();
        for key in keys.iter() {
            let Some(queue) = self.queues.get_async(key).await.map(|q| q.clone()) else { continue };
            queue.fastrand_shuffle();
        }
    }
}

struct FindWaiterGuard<'a> {
    count: &'a AtomicUsize,
}

impl Drop for FindWaiterGuard<'_> {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::Release);
    }
}

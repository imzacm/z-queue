use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_utils::CachePadded;
use parking_lot::Mutex;

use crate::container::ContainerTrait;

#[derive(Debug)]
pub struct VecDequeContainer<T> {
    queue: CachePadded<Mutex<VecDeque<T>>>,
    len: AtomicUsize,
    capacity: Option<usize>,
}

impl<T> VecDequeContainer<T> {
    pub fn new(capacity: Option<usize>) -> Self {
        let cap = capacity.unwrap_or(0);
        Self {
            queue: CachePadded::new(Mutex::new(VecDeque::with_capacity(cap))),
            len: AtomicUsize::new(0),
            capacity,
        }
    }
}

impl<T> ContainerTrait<T> for VecDequeContainer<T> {
    fn len(&self) -> usize {
        self.len.load(Ordering::Relaxed)
    }

    fn capacity(&self) -> Option<usize> {
        self.capacity
    }

    fn clear(&self) -> usize {
        let mut lock = self.queue.lock();
        lock.clear();
        self.len.swap(0, Ordering::AcqRel)
    }

    fn push(&self, item: T) -> Result<(), T> {
        let mut lock = self.queue.lock();
        if self.is_full() {
            return Err(item);
        }
        lock.push_back(item);
        self.len.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn pop(&self) -> Option<T> {
        let mut lock = self.queue.lock();
        let item = lock.pop_front();
        if item.is_some() {
            self.len.fetch_sub(1, Ordering::Relaxed);
        }
        item
    }

    fn find_pop<F>(&self, find_fn: F) -> Option<T>
    where
        F: FnMut(&T) -> bool,
    {
        let mut lock = self.queue.lock();
        let index = lock.iter().position(find_fn)?;
        let item = lock.remove(index)?;
        self.len.fetch_sub(1, Ordering::Relaxed);
        Some(item)
    }

    fn retain<F>(&self, retain_fn: F) -> usize
    where
        F: FnMut(&T) -> bool,
    {
        let mut lock = self.queue.lock();
        let old_len = lock.len();
        lock.retain(retain_fn);
        let new_len = lock.len();
        self.len.store(new_len, Ordering::Relaxed);
        old_len - new_len
    }

    fn retain_into<F>(&self, retain_fn: F, removed: &mut Vec<T>)
    where
        F: FnMut(&T) -> bool,
    {
        let mut lock = self.queue.lock();
        vec_deque_retain_into(&mut lock, removed, retain_fn);
        self.len.store(lock.len(), Ordering::Relaxed);
    }

    #[cfg(feature = "rand")]
    fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R) {
        use rand::seq::SliceRandom;

        self.queue.lock().make_contiguous().shuffle(rng);
    }

    #[cfg(feature = "fastrand")]
    fn fastrand_shuffle(&self) {
        fastrand::shuffle(self.queue.lock().make_contiguous());
    }
}

fn vec_deque_retain_into<T, F>(queue: &mut VecDeque<T>, target: &mut Vec<T>, mut retain_fn: F)
where
    F: FnMut(&T) -> bool,
{
    let original_len = queue.len();

    for _ in 0..original_len {
        // Evaluate the predicate before popping to prevent losing
        // the current item if `retain_fn` panics.
        let keep = if let Some(front) = queue.front() {
            retain_fn(front)
        } else {
            break;
        };

        // Safe to unwrap because we just verified `front` is `Some`
        let item = queue.pop_front().unwrap();

        if keep {
            // Kept items are cycled to the back of the queue
            queue.push_back(item);
        } else {
            // Rejected items are moved to the target Vec
            target.push(item);
        }
    }
}

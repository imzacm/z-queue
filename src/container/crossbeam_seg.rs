use alloc::vec::Vec;
use core::num::NonZeroUsize;

use crossbeam_queue::SegQueue;
use parking_lot::RwLock;

use crate::container::{Container, CreateUnbounded};

#[derive(Debug)]
pub struct CrossbeamSegQueue<T> {
    queue: RwLock<SegQueue<T>>,
}

impl<T> CreateUnbounded for CrossbeamSegQueue<T> {
    fn new_unbounded() -> Self {
        Self { queue: RwLock::new(SegQueue::new()) }
    }
}

impl<T> Container for CrossbeamSegQueue<T> {
    type Item = T;

    fn len(&self) -> usize {
        self.queue.read().len()
    }

    fn capacity(&self) -> Option<NonZeroUsize> {
        None
    }

    fn clear(&self) -> usize {
        let lock = self.queue.read();
        let mut removed = 0;
        while lock.pop().is_some() {
            removed += 1;
        }
        removed
    }

    fn push(&self, item: T) -> Result<(), T> {
        self.queue.read().push(item);
        Ok(())
    }

    fn pop(&self) -> Option<T> {
        self.queue.read().pop()
    }

    fn find_pop<F>(&self, mut find_fn: F) -> Option<T>
    where
        F: FnMut(&T) -> bool,
    {
        let lock = self.queue.write();
        for _ in 0..lock.len() {
            let Some(item) = lock.pop() else { break };
            if find_fn(&item) {
                return Some(item);
            }
            lock.push(item);
        }
        None
    }

    fn retain<F>(&self, mut retain_fn: F) -> usize
    where
        F: FnMut(&T) -> bool,
    {
        let lock = self.queue.write();
        let mut removed = 0;
        for _ in 0..lock.len() {
            let Some(item) = lock.pop() else { break };
            if retain_fn(&item) {
                lock.push(item);
            } else {
                removed += 1;
            }
        }
        removed
    }

    fn retain_into<F>(&self, mut retain_fn: F, removed: &mut Vec<T>)
    where
        F: FnMut(&T) -> bool,
    {
        let lock = self.queue.write();
        for _ in 0..lock.len() {
            let Some(item) = lock.pop() else { break };
            if retain_fn(&item) {
                lock.push(item);
            } else {
                removed.push(item);
            }
        }
    }

    #[cfg(feature = "rand")]
    fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R) {
        use rand::seq::SliceRandom;

        let lock = self.queue.write();
        let mut items = Vec::with_capacity(lock.len());
        while let Some(item) = lock.pop() {
            items.push(item);
        }
        items.shuffle(rng);
        for item in items {
            lock.push(item);
        }
    }

    #[cfg(feature = "fastrand")]
    fn fastrand_shuffle(&self) {
        let lock = self.queue.write();
        let mut items = Vec::with_capacity(lock.len());
        while let Some(item) = lock.pop() {
            items.push(item);
        }
        fastrand::shuffle(&mut items);
        for item in items {
            lock.push(item);
        }
    }
}

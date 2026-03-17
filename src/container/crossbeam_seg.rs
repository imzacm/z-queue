use alloc::vec::Vec;
use core::num::NonZeroUsize;

use crossbeam_queue::SegQueue;

use crate::container::{Container, CreateUnbounded};

#[derive(Debug)]
pub struct CrossbeamSegQueue<T> {
    queue: SegQueue<T>,
}

impl<T> CreateUnbounded for CrossbeamSegQueue<T> {
    fn new_unbounded() -> Self {
        Self { queue: SegQueue::new() }
    }
}

impl<T> Container for CrossbeamSegQueue<T> {
    type Item = T;

    fn len(&self) -> usize {
        self.queue.len()
    }

    fn capacity(&self) -> Option<NonZeroUsize> {
        None
    }

    fn clear(&self) -> usize {
        let mut removed = 0;
        while self.queue.pop().is_some() {
            removed += 1;
        }
        removed
    }

    fn push(&self, item: T) -> Result<(), T> {
        self.queue.push(item);
        Ok(())
    }

    fn pop(&self) -> Option<T> {
        self.queue.pop()
    }

    fn find_pop<F>(&self, mut find_fn: F) -> Option<T>
    where
        F: FnMut(&T) -> bool,
    {
        for _ in 0..self.queue.len() {
            let Some(item) = self.queue.pop() else { break };
            if find_fn(&item) {
                return Some(item);
            }
            self.queue.push(item);
        }
        None
    }

    fn retain<F>(&self, mut retain_fn: F) -> usize
    where
        F: FnMut(&T) -> bool,
    {
        let mut removed = 0;
        for _ in 0..self.queue.len() {
            let Some(item) = self.queue.pop() else { break };
            if retain_fn(&item) {
                self.queue.push(item);
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
        for _ in 0..self.queue.len() {
            let Some(item) = self.queue.pop() else { break };
            if retain_fn(&item) {
                self.queue.push(item);
            } else {
                removed.push(item);
            }
        }
    }

    fn visit<F>(&self, mut visit_fn: F)
    where
        F: FnMut(&Self::Item),
    {
        for _ in 0..self.queue.len() {
            let Some(item) = self.queue.pop() else { break };
            visit_fn(&item);
            self.queue.push(item);
        }
    }

    #[cfg(feature = "rand")]
    fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R) {
        use rand::seq::SliceRandom;

        let mut items = Vec::with_capacity(self.queue.len());
        while let Some(item) = self.queue.pop() {
            items.push(item);
        }
        items.shuffle(rng);
        for item in items {
            self.queue.push(item);
        }
    }

    #[cfg(feature = "fastrand")]
    fn fastrand_shuffle(&self) {
        let mut items = Vec::with_capacity(self.queue.len());
        while let Some(item) = self.queue.pop() {
            items.push(item);
        }
        fastrand::shuffle(&mut items);
        for item in items {
            self.queue.push(item);
        }
    }
}

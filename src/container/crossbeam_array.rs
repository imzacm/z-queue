use alloc::vec::Vec;

use crossbeam_queue::ArrayQueue;
use parking_lot::RwLock;

use crate::container::ContainerTrait;

#[derive(Debug)]
pub struct CrossbeamArrayContainer<T> {
    queue: RwLock<ArrayQueue<T>>,
}

impl<T> CrossbeamArrayContainer<T> {
    pub fn new(capacity: usize) -> Self {
        Self { queue: RwLock::new(ArrayQueue::new(capacity)) }
    }
}

impl<T> ContainerTrait<T> for CrossbeamArrayContainer<T> {
    fn len(&self) -> usize {
        self.queue.read().len()
    }

    fn capacity(&self) -> Option<usize> {
        Some(self.queue.read().capacity())
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
        self.queue.read().push(item)
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
            if lock.push(item).is_err() {
                panic!("ArrayQueue container is full");
            }
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
                if lock.push(item).is_err() {
                    panic!("ArrayQueue container is full");
                }
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
                if lock.push(item).is_err() {
                    panic!("ArrayQueue container is full");
                }
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
            if lock.push(item).is_err() {
                panic!("ArrayQueue container is full");
            }
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
            if lock.push(item).is_err() {
                panic!("ArrayQueue container is full");
            }
        }
    }
}

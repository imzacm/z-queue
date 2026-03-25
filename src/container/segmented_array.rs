use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::num::NonZeroUsize;
use core::sync::atomic::{AtomicUsize, Ordering};

use arrayvec::ArrayVec;
use crossbeam_utils::CachePadded;
use parking_lot::Mutex;

use crate::container::{Container, CreateBounded, CreateUnbounded};

#[derive(Debug)]
pub struct SegmentedArray<T, const SEGMENT_SIZE: usize> {
    queue: CachePadded<Mutex<VecDeque<ArrayVec<T, SEGMENT_SIZE>>>>,
    len: CachePadded<AtomicUsize>,
    capacity: Option<NonZeroUsize>,
}

impl<T, const SEGMENT_SIZE: usize> CreateBounded for SegmentedArray<T, SEGMENT_SIZE> {
    fn new_bounded(capacity: NonZeroUsize) -> Self {
        let block_capacity = capacity.get().div_ceil(SEGMENT_SIZE);
        let mut queue = VecDeque::with_capacity(block_capacity);
        queue.push_back(ArrayVec::new());
        Self {
            queue: CachePadded::new(Mutex::new(queue)),
            len: CachePadded::new(AtomicUsize::new(0)),
            capacity: Some(capacity),
        }
    }
}

impl<T, const SEGMENT_SIZE: usize> CreateUnbounded for SegmentedArray<T, SEGMENT_SIZE> {
    fn new_unbounded() -> Self {
        let mut queue = VecDeque::new();
        queue.push_back(ArrayVec::new());
        Self {
            queue: CachePadded::new(Mutex::new(queue)),
            len: CachePadded::new(AtomicUsize::new(0)),
            capacity: None,
        }
    }
}

impl<T, const SEGMENT_SIZE: usize> Container for SegmentedArray<T, SEGMENT_SIZE> {
    type Item = T;

    #[inline(always)]
    fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }

    #[inline(always)]
    fn capacity(&self) -> Option<NonZeroUsize> {
        self.capacity
    }

    #[inline(always)]
    fn clear(&self) -> usize {
        let mut lock = self.queue.lock();
        lock.clear();
        lock.push_back(ArrayVec::new());
        self.len.swap(0, Ordering::Acquire)
    }

    fn push(&self, item: T) -> Result<(), T> {
        if self.is_full() {
            return Err(item);
        }

        let mut lock = self.queue.lock();
        let back = lock.back_mut().expect("Queue always has at least one block");

        if back.is_full() {
            // The current tail is full, create a new block
            let mut new_block = ArrayVec::new();
            new_block.push(item);
            lock.push_back(new_block);
        } else {
            // Push into the existing tail block
            back.push(item);
        }

        self.len.fetch_add(1, Ordering::Release);
        Ok(())
    }

    fn pop(&self) -> Option<T> {
        let mut lock = self.queue.lock();
        loop {
            let front = lock.front_mut().expect("Queue always has at least one block");

            if !front.is_empty() {
                self.len.fetch_sub(1, Ordering::Release);
                return Some(front.remove(0));
            }

            if lock.len() == 1 {
                return None;
            }

            lock.pop_front();
        }
    }

    fn find_pop<F>(&self, mut find_fn: F) -> Option<T>
    where
        F: FnMut(&T) -> bool,
    {
        let mut lock = self.queue.lock();

        let mut item = None;
        let mut remove_block_index = None;

        for (block_index, block) in lock.iter_mut().enumerate() {
            let index = block.iter().position(&mut find_fn);
            let Some(index) = index else { continue };

            item = Some(block.remove(index));
            if block.is_empty() {
                remove_block_index = Some(block_index);
            }

            break;
        }

        if let Some(index) = remove_block_index
            && lock.len() > 1
        {
            lock.remove(index);
        }

        if item.is_some() {
            self.len.fetch_sub(1, Ordering::Release);
        }

        item
    }

    fn retain<F>(&self, mut retain_fn: F) -> usize
    where
        F: FnMut(&T) -> bool,
    {
        let mut lock = self.queue.lock();

        let mut removed = 0;

        lock.retain_mut(|block| {
            let old_len = block.len();
            block.retain(|v| retain_fn(v));
            let new_len = block.len();

            let delta = old_len - new_len;
            removed += delta;

            new_len != 0
        });

        if lock.is_empty() {
            lock.push_back(ArrayVec::new());
        }

        self.len.fetch_sub(removed, Ordering::Release);
        removed
    }

    fn retain_into<F>(&self, mut retain_fn: F, removed: &mut Vec<T>)
    where
        F: FnMut(&T) -> bool,
    {
        let mut lock = self.queue.lock();

        let old_len = removed.len();
        lock.retain_mut(|block| {
            array_vec_retain_into(block, removed, &mut retain_fn);
            !block.is_empty()
        });

        if lock.is_empty() {
            lock.push_back(ArrayVec::new());
        }

        let new_len = removed.len();
        let removed = new_len - old_len;
        self.len.fetch_sub(removed, Ordering::Release);
    }

    fn visit<F>(&self, mut visit_fn: F)
    where
        F: FnMut(&Self::Item),
    {
        let lock = self.queue.lock();
        for block in lock.iter() {
            for item in block.iter() {
                visit_fn(item);
            }
        }
    }

    #[cfg(feature = "rand")]
    fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R) {
        use rand::seq::SliceRandom;

        self.queue.lock().make_contiguous().shuffle(rng);
        for block in self.queue.lock().iter_mut() {
            block.shuffle(rng);
        }
    }

    #[cfg(feature = "fastrand")]
    fn fastrand_shuffle(&self) {
        fastrand::shuffle(self.queue.lock().make_contiguous());
        for block in self.queue.lock().iter_mut() {
            fastrand::shuffle(block);
        }
    }
}

pub fn array_vec_retain_into<T, F, const CAP: usize>(
    queue: &mut ArrayVec<T, CAP>,
    target: &mut Vec<T>,
    mut retain_fn: F,
) where
    F: FnMut(&T) -> bool,
{
    // Take ownership of all items, leaving `queue` temporarily empty.
    let temp_iter = core::mem::replace(queue, ArrayVec::new()).into_iter();

    // A drop guard to guarantee panic safety. If `retain_fn` or `target.push`
    // panics, the guard catches the unwind and puts all unprocessed items
    // safely back into the original `queue`.
    struct PanicGuard<'a, T, const CAP: usize> {
        queue: &'a mut ArrayVec<T, CAP>,
        iter: arrayvec::IntoIter<T, CAP>,
    }

    impl<'a, T, const CAP: usize> Drop for PanicGuard<'a, T, CAP> {
        fn drop(&mut self) {
            // If the loop panics, push remaining unexamined items back.
            for item in self.iter.by_ref() {
                self.queue.push(item);
            }
        }
    }

    let mut guard = PanicGuard { queue, iter: temp_iter };

    for item in &mut guard.iter {
        if retain_fn(&item) {
            // Safe: We know we will never exceed CAP because we are only
            // returning items that originally fit in the ArrayVec.
            guard.queue.push(item);
        } else {
            target.push(item);
        }
    }
}

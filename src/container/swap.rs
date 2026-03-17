use alloc::vec::Vec;
use core::num::NonZeroUsize;
use core::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_utils::CachePadded;

use crate::container::{Container, CreateBounded, CreateUnbounded};

#[derive(Debug)]
pub struct Swap<C, const N: usize> {
    containers: [C; N],
    push_index: CachePadded<AtomicUsize>,
    pop_index: CachePadded<AtomicUsize>,
    capacity: Option<NonZeroUsize>,
}

impl<C, const N: usize> CreateBounded for Swap<C, N>
where
    C: CreateBounded,
{
    fn new_bounded(capacity: NonZeroUsize) -> Self {
        let container_capacity = capacity.get().div_ceil(N);
        let container_capacity = NonZeroUsize::new(container_capacity).unwrap();
        let containers = core::array::from_fn(|_| C::new_bounded(container_capacity));

        Self {
            containers,
            push_index: CachePadded::new(AtomicUsize::new(0)),
            pop_index: CachePadded::new(AtomicUsize::new(0)),
            capacity: Some(capacity),
        }
    }
}

impl<C, const N: usize> CreateUnbounded for Swap<C, N>
where
    C: CreateUnbounded,
{
    fn new_unbounded() -> Self {
        let containers = core::array::from_fn(|_| C::new_unbounded());
        Self {
            containers,
            push_index: CachePadded::new(AtomicUsize::new(0)),
            pop_index: CachePadded::new(AtomicUsize::new(0)),
            capacity: None,
        }
    }
}

impl<C, const N: usize> Container for Swap<C, N>
where
    C: Container,
{
    type Item = C::Item;

    fn len(&self) -> usize {
        self.containers.iter().map(|c| c.len()).sum()
    }

    fn capacity(&self) -> Option<NonZeroUsize> {
        self.capacity
    }

    fn clear(&self) -> usize {
        let mut removed = 0;
        for container in &self.containers {
            removed += container.clear();
        }
        removed
    }

    fn push(&self, mut item: Self::Item) -> Result<(), Self::Item> {
        if self.is_full() {
            return Err(item);
        }

        let start_index = self.push_index.fetch_add(1, Ordering::Release);
        for index in 0..N {
            let index = (start_index + index) % N;
            let container = &self.containers[index];
            match container.push(item) {
                Ok(()) => return Ok(()),
                Err(v) => item = v,
            }
        }

        Err(item)
    }

    fn pop(&self) -> Option<Self::Item> {
        let start_index = self.pop_index.fetch_add(1, Ordering::Release);
        for index in 0..N {
            let index = (start_index + index) % N;
            let container = &self.containers[index];
            if let Some(item) = container.pop() {
                return Some(item);
            }
        }
        None
    }

    fn find_pop<F>(&self, mut find_fn: F) -> Option<Self::Item>
    where
        F: FnMut(&Self::Item) -> bool,
    {
        let start_index = self.pop_index.fetch_add(1, Ordering::Relaxed);
        for index in 0..N {
            let index = (start_index + index) % N;
            let container = &self.containers[index];
            if let Some(item) = container.find_pop(&mut find_fn) {
                return Some(item);
            }
        }
        None
    }

    fn retain<F>(&self, mut retain_fn: F) -> usize
    where
        F: FnMut(&Self::Item) -> bool,
    {
        let mut removed = 0;
        for container in &self.containers {
            removed += container.retain(&mut retain_fn);
        }
        removed
    }

    fn retain_into<F>(&self, mut retain_fn: F, removed: &mut Vec<Self::Item>)
    where
        F: FnMut(&Self::Item) -> bool,
    {
        for container in &self.containers {
            container.retain_into(&mut retain_fn, removed);
        }
    }

    #[cfg(feature = "rand")]
    fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R) {
        for container in &self.containers {
            container.rand_shuffle(rng);
        }
    }

    #[cfg(feature = "fastrand")]
    fn fastrand_shuffle(&self) {
        for container in &self.containers {
            container.fastrand_shuffle();
        }
    }
}

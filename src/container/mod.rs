#[cfg(feature = "crossbeam-queue")]
mod crossbeam_array;
#[cfg(feature = "crossbeam-queue")]
mod crossbeam_seg;
#[cfg(feature = "segmented-array")]
mod segmented_array;
mod swap;
mod vec_deque;

use alloc::vec::Vec;
use core::num::NonZeroUsize;

#[cfg(feature = "crossbeam-queue")]
pub use self::crossbeam_array::CrossbeamArrayQueue;
#[cfg(feature = "crossbeam-queue")]
pub use self::crossbeam_seg::CrossbeamSegQueue;
#[cfg(feature = "segmented-array")]
pub use self::segmented_array::SegmentedArray;
pub use self::swap::Swap;
pub use self::vec_deque::VecDeque;

pub trait CreateBounded: Container + Sized {
    fn new_bounded(capacity: NonZeroUsize) -> Self;
}

pub trait CreateUnbounded: Container + Sized {
    fn new_unbounded() -> Self;
}

pub trait Container {
    type Item;

    fn len(&self) -> usize;

    fn capacity(&self) -> Option<NonZeroUsize>;

    #[inline(always)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    fn is_full(&self) -> bool {
        self.capacity().is_some_and(|cap| self.len() >= cap.get())
    }

    fn clear(&self) -> usize;

    fn push(&self, item: Self::Item) -> Result<(), Self::Item>;

    fn pop(&self) -> Option<Self::Item>;

    fn find_pop<F>(&self, find_fn: F) -> Option<Self::Item>
    where
        F: FnMut(&Self::Item) -> bool;

    fn retain<F>(&self, retain_fn: F) -> usize
    where
        F: FnMut(&Self::Item) -> bool;

    fn retain_into<F>(&self, retain_fn: F, removed: &mut Vec<Self::Item>)
    where
        F: FnMut(&Self::Item) -> bool;

    /// Visit each item in the container. Order is not guaranteed.
    fn visit<F>(&self, visit_fn: F)
    where
        F: FnMut(&Self::Item);

    #[cfg(feature = "rand")]
    fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R);

    #[cfg(feature = "fastrand")]
    fn fastrand_shuffle(&self);
}

#[cfg(feature = "crossbeam-queue")]
mod state {
    use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use crossbeam_utils::CachePadded;

    #[derive(Default, Debug)]
    pub struct ContainerState {
        active_pushes: CachePadded<AtomicUsize>,
        active_pops: CachePadded<AtomicUsize>,
        suspend: CachePadded<AtomicBool>,
    }

    impl ContainerState {
        pub fn push_pop<'v>(
            value: &'v AtomicUsize,
            suspend: &AtomicBool,
        ) -> ContainerStateGuard<'v> {
            let backoff = crossbeam_utils::Backoff::new();
            loop {
                // Avoid dirtying the cache line if already suspended.
                if suspend.load(Ordering::Relaxed) {
                    backoff.snooze();
                    continue;
                }

                value.fetch_add(1, Ordering::SeqCst);

                if !suspend.load(Ordering::SeqCst) {
                    return ContainerStateGuard { value };
                }

                value.fetch_sub(1, Ordering::Release);

                while suspend.load(Ordering::Relaxed) {
                    backoff.snooze();
                }
            }
        }

        pub fn push(&self) -> ContainerStateGuard<'_> {
            Self::push_pop(&self.active_pushes, &self.suspend)
        }

        pub fn pop(&self) -> ContainerStateGuard<'_> {
            Self::push_pop(&self.active_pops, &self.suspend)
        }

        pub fn suspend(&self) -> ContainerSuspendGuard<'_> {
            let backoff = crossbeam_utils::Backoff::new();

            while self
                .suspend
                .compare_exchange_weak(false, true, Ordering::SeqCst, Ordering::Relaxed)
                .is_err()
            {
                backoff.snooze();
            }

            let backoff = crossbeam_utils::Backoff::new();

            while self.active_pushes.load(Ordering::SeqCst) > 0
                || self.active_pops.load(Ordering::SeqCst) > 0
            {
                backoff.snooze();
            }

            ContainerSuspendGuard { value: &self.suspend }
        }
    }

    #[derive(Debug)]
    pub struct ContainerStateGuard<'a> {
        value: &'a AtomicUsize,
    }

    impl Drop for ContainerStateGuard<'_> {
        fn drop(&mut self) {
            self.value.fetch_sub(1, Ordering::Release);
        }
    }

    #[derive(Debug)]
    pub struct ContainerSuspendGuard<'a> {
        value: &'a AtomicBool,
    }

    impl Drop for ContainerSuspendGuard<'_> {
        fn drop(&mut self) {
            self.value.store(false, Ordering::Release);
        }
    }
}

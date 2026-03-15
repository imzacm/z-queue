mod crossbeam_array;
mod crossbeam_seg;
mod segmented_array;
mod swap;
mod vec_deque;
#[cfg(feature = "wal")]
mod wal;

use alloc::vec::Vec;
use core::num::NonZeroUsize;

pub use self::crossbeam_array::CrossbeamArrayQueue;
pub use self::crossbeam_seg::CrossbeamSegQueue;
pub use self::segmented_array::SegmentedArray;
pub use self::swap::Swap;
pub use self::vec_deque::VecDeque;
#[cfg(feature = "wal")]
pub use self::wal::Wal;

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

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn is_full(&self) -> bool {
        self.capacity().is_none_or(|cap| self.len() >= cap.get())
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

    #[cfg(feature = "rand")]
    fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R);

    #[cfg(feature = "fastrand")]
    fn fastrand_shuffle(&self);
}

pub mod crossbeam_array;
pub mod crossbeam_seg;
pub mod segmented_array;
pub mod vec_deque;

use alloc::vec::Vec;

pub const SEGMENT_SIZE: usize = 64;

pub trait ContainerTrait<T> {
    fn len(&self) -> usize;
    fn capacity(&self) -> Option<usize>;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn is_full(&self) -> bool {
        self.capacity().is_none_or(|cap| self.len() >= cap)
    }

    fn clear(&self) -> usize;
    fn push(&self, item: T) -> Result<(), T>;
    fn pop(&self) -> Option<T>;

    fn find_pop<F>(&self, find_fn: F) -> Option<T>
    where
        F: FnMut(&T) -> bool;

    fn retain<F>(&self, retain_fn: F) -> usize
    where
        F: FnMut(&T) -> bool;

    fn retain_into<F>(&self, retain_fn: F, removed: &mut Vec<T>)
    where
        F: FnMut(&T) -> bool;

    #[cfg(feature = "rand")]
    fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R);

    #[cfg(feature = "fastrand")]
    fn fastrand_shuffle(&self);
}

#[derive(Debug)]
pub enum Container<T> {
    VecDeque(vec_deque::VecDequeContainer<T>),
    SegmentedArray(segmented_array::SegmentedArrayContainer<T, SEGMENT_SIZE>),
    CrossbeamArray(crossbeam_array::CrossbeamArrayContainer<T>),
    CrossbeamSeg(crossbeam_seg::CrossbeamSeqContainer<T>),
}

impl<T> Container<T> {
    pub fn new_vec_dequeue(capacity: Option<usize>) -> Self {
        Self::VecDeque(vec_deque::VecDequeContainer::new(capacity))
    }

    pub fn new_segmented_array(capacity: Option<usize>) -> Self {
        Self::SegmentedArray(segmented_array::SegmentedArrayContainer::new(capacity))
    }

    pub fn new_crossbeam_array(capacity: usize) -> Self {
        Self::CrossbeamArray(crossbeam_array::CrossbeamArrayContainer::new(capacity))
    }

    pub fn new_crossbeam_seg() -> Self {
        Self::CrossbeamSeg(crossbeam_seg::CrossbeamSeqContainer::new())
    }
}

impl<T> ContainerTrait<T> for Container<T> {
    fn len(&self) -> usize {
        match self {
            Self::VecDeque(c) => c.len(),
            Self::SegmentedArray(c) => c.len(),
            Self::CrossbeamArray(c) => c.len(),
            Self::CrossbeamSeg(c) => c.len(),
        }
    }

    fn capacity(&self) -> Option<usize> {
        match self {
            Self::VecDeque(c) => c.capacity(),
            Self::SegmentedArray(c) => c.capacity(),
            Self::CrossbeamArray(c) => c.capacity(),
            Self::CrossbeamSeg(c) => c.capacity(),
        }
    }

    fn clear(&self) -> usize {
        match self {
            Self::VecDeque(c) => c.clear(),
            Self::SegmentedArray(c) => c.clear(),
            Self::CrossbeamArray(c) => c.clear(),
            Self::CrossbeamSeg(c) => c.clear(),
        }
    }

    fn push(&self, item: T) -> Result<(), T> {
        match self {
            Self::VecDeque(c) => c.push(item),
            Self::SegmentedArray(c) => c.push(item),
            Self::CrossbeamArray(c) => c.push(item),
            Self::CrossbeamSeg(c) => c.push(item),
        }
    }

    fn pop(&self) -> Option<T> {
        match self {
            Self::VecDeque(c) => c.pop(),
            Self::SegmentedArray(c) => c.pop(),
            Self::CrossbeamArray(c) => c.pop(),
            Self::CrossbeamSeg(c) => c.pop(),
        }
    }

    fn find_pop<F>(&self, find_fn: F) -> Option<T>
    where
        F: FnMut(&T) -> bool,
    {
        match self {
            Self::VecDeque(c) => c.find_pop(find_fn),
            Self::SegmentedArray(c) => c.find_pop(find_fn),
            Self::CrossbeamArray(c) => c.find_pop(find_fn),
            Self::CrossbeamSeg(c) => c.find_pop(find_fn),
        }
    }

    fn retain<F>(&self, retain_fn: F) -> usize
    where
        F: FnMut(&T) -> bool,
    {
        match self {
            Self::VecDeque(c) => c.retain(retain_fn),
            Self::SegmentedArray(c) => c.retain(retain_fn),
            Self::CrossbeamArray(c) => c.retain(retain_fn),
            Self::CrossbeamSeg(c) => c.retain(retain_fn),
        }
    }

    fn retain_into<F>(&self, retain_fn: F, removed: &mut Vec<T>)
    where
        F: FnMut(&T) -> bool,
    {
        match self {
            Self::VecDeque(c) => c.retain_into(retain_fn, removed),
            Self::SegmentedArray(c) => c.retain_into(retain_fn, removed),
            Self::CrossbeamArray(c) => c.retain_into(retain_fn, removed),
            Self::CrossbeamSeg(c) => c.retain_into(retain_fn, removed),
        }
    }

    #[cfg(feature = "rand")]
    fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R) {
        match self {
            Self::VecDeque(c) => c.rand_shuffle(rng),
            Self::SegmentedArray(c) => c.rand_shuffle(rng),
            Self::CrossbeamArray(c) => c.rand_shuffle(rng),
            Self::CrossbeamSeg(c) => c.rand_shuffle(rng),
        }
    }

    #[cfg(feature = "fastrand")]
    fn fastrand_shuffle(&self) {
        match self {
            Self::VecDeque(c) => c.fastrand_shuffle(),
            Self::SegmentedArray(c) => c.fastrand_shuffle(),
            Self::CrossbeamArray(c) => c.fastrand_shuffle(),
            Self::CrossbeamSeg(c) => c.fastrand_shuffle(),
        }
    }
}

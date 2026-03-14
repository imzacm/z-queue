use alloc::collections::VecDeque;
use alloc::vec::Vec;

use arrayvec::ArrayVec;

#[derive(Debug)]
pub enum Container<T, const SEGMENT_SIZE: usize> {
    VecDeque { queue: VecDeque<T> },
    SegmentedArray { queue: VecDeque<ArrayVec<T, SEGMENT_SIZE>> },
}

impl<T, const SEGMENT_SIZE: usize> Container<T, SEGMENT_SIZE> {
    pub fn new_vec_deque(capacity: usize) -> Self {
        Self::VecDeque { queue: VecDeque::with_capacity(capacity) }
    }

    pub fn new_segmented_array() -> Self {
        let mut queue = VecDeque::new();
        queue.push_back(ArrayVec::new());
        Self::SegmentedArray { queue }
    }

    pub fn clear(&mut self) {
        match self {
            Self::VecDeque { queue, .. } => queue.clear(),
            Self::SegmentedArray { queue, .. } => {
                queue.clear();
                queue.push_back(ArrayVec::new());
            }
        }
    }

    pub fn push(&mut self, item: T) {
        match self {
            Self::VecDeque { queue, .. } => {
                queue.push_back(item);
            }
            Self::SegmentedArray { queue } => {
                let back = queue.back_mut().expect("Queue always has at least one block");

                if back.is_full() {
                    // The current tail is full, create a new block
                    let mut new_block = ArrayVec::new();
                    new_block.push(item);
                    queue.push_back(new_block);
                } else {
                    // Push into the existing tail block
                    back.push(item);
                }
            }
        }
    }

    pub fn pop(&mut self) -> Option<T> {
        match self {
            Self::VecDeque { queue, .. } => queue.pop_front(),
            Self::SegmentedArray { queue } => {
                let front = queue.front_mut().expect("Queue always has at least one block");

                if !front.is_empty() {
                    return Some(front.remove(0));
                }

                if queue.len() > 1 {
                    queue.pop_front();
                    // Recurse once to pop from the new front block
                    return self.pop();
                }

                None
            }
        }
    }

    pub fn find_pop<F>(&mut self, mut find_fn: F) -> Option<T>
    where
        F: FnMut(&T) -> bool,
    {
        match self {
            Self::VecDeque { queue, .. } => {
                let index = queue.iter().position(find_fn)?;
                queue.remove(index)
            }
            Self::SegmentedArray { queue } => {
                let mut item = None;
                let mut remove_block_index = None;

                for (block_index, block) in queue.iter_mut().enumerate() {
                    let index = block.iter().position(&mut find_fn);
                    let Some(index) = index else { continue };

                    item = Some(block.remove(index));
                    if block.is_empty() {
                        remove_block_index = Some(block_index);
                    }

                    break;
                }

                if let Some(index) = remove_block_index
                    && queue.len() > 1
                {
                    queue.remove(index);
                }

                item
            }
        }
    }

    pub fn retain<F>(&mut self, mut retain_fn: F) -> usize
    where
        F: FnMut(&T) -> bool,
    {
        match self {
            Self::VecDeque { queue, .. } => {
                let old_len = queue.len();
                queue.retain(retain_fn);
                let new_len = queue.len();
                old_len - new_len
            }
            Self::SegmentedArray { queue } => {
                let mut removed = 0;

                queue.retain_mut(|block| {
                    let old_len = block.len();
                    block.retain(|v| retain_fn(v));
                    let new_len = block.len();

                    let delta = old_len - new_len;
                    removed += delta;

                    new_len != 0
                });

                if queue.is_empty() {
                    queue.push_back(ArrayVec::new());
                }

                removed
            }
        }
    }

    pub fn retain_into<F>(&mut self, mut retain_fn: F, removed: &mut Vec<T>)
    where
        F: FnMut(&T) -> bool,
    {
        match self {
            Self::VecDeque { queue, .. } => {
                vec_dequeue_retain_into(queue, removed, retain_fn);
            }
            Self::SegmentedArray { queue } => {
                queue.retain_mut(|block| {
                    array_vec_retain_into(block, removed, &mut retain_fn);
                    !block.is_empty()
                });

                if queue.is_empty() {
                    queue.push_back(ArrayVec::new());
                }
            }
        }
    }
}

fn vec_dequeue_retain_into<T, F>(queue: &mut VecDeque<T>, target: &mut Vec<T>, mut retain_fn: F)
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

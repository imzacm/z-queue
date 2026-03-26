//! Type aliases and functions for using the default containers.
//!
//! These are (based on benchmarks):
//!
//! - Bounded: [`CrossbeamArrayQueue`]
//! - Unbounded: [`VecDeque`]

use core::num::NonZeroUsize;

#[cfg(feature = "crossbeam-queue")]
use crate::container::CrossbeamArrayQueue;
use crate::container::VecDeque;
use crate::{Receiver, Sender, ZQueue};

#[cfg(feature = "crossbeam-queue")]
pub type BoundedQueue<T> = ZQueue<CrossbeamArrayQueue<T>>;
pub type UnboundedQueue<T> = ZQueue<VecDeque<T>>;

#[cfg(feature = "crossbeam-queue")]
pub type BoundedSender<T> = Sender<CrossbeamArrayQueue<T>>;
#[cfg(feature = "crossbeam-queue")]
pub type BoundedReceiver<T> = Receiver<CrossbeamArrayQueue<T>>;

#[cfg(not(feature = "crossbeam-queue"))]
pub type BoundedSender<T> = Sender<VecDeque<T>>;
#[cfg(not(feature = "crossbeam-queue"))]
pub type BoundedReceiver<T> = Receiver<VecDeque<T>>;

pub type UnboundedSender<T> = Sender<VecDeque<T>>;
pub type UnboundedReceiver<T> = Receiver<VecDeque<T>>;

#[cfg(all(feature = "stream", feature = "crossbeam-queue"))]
pub type BoundedRecvStream<T> = crate::RecvStream<CrossbeamArrayQueue<T>>;

#[cfg(all(feature = "stream", not(feature = "crossbeam-queue")))]
pub type BoundedRecvStream<T> = crate::RecvStream<VecDeque<T>>;

#[cfg(feature = "stream")]
pub type UnboundedRecvStream<T> = crate::RecvStream<VecDeque<T>>;

pub fn bounded<T>(capacity: NonZeroUsize) -> (BoundedSender<T>, BoundedReceiver<T>) {
    crate::bounded(capacity)
}

pub fn unbounded<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    crate::unbounded()
}

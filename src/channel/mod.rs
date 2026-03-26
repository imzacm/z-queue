mod receiver;
mod recv_iter;
#[cfg(feature = "stream")]
mod recv_stream;
mod recv_try_iter;
mod sender;

#[cfg(not(feature = "triomphe"))]
use alloc::sync::Arc;
use core::num::NonZeroUsize;
use core::sync::atomic::AtomicUsize;

use crossbeam_utils::CachePadded;
#[cfg(feature = "triomphe")]
use triomphe::Arc;

pub use self::receiver::Receiver;
pub use self::recv_iter::RecvIter;
#[cfg(feature = "stream")]
pub use self::recv_stream::RecvStream;
pub use self::recv_try_iter::RecvTryIter;
pub use self::sender::Sender;
use crate::ZQueue;
use crate::container::{Container, CreateBounded, CreateUnbounded};

pub fn bounded<C>(capacity: NonZeroUsize) -> (Sender<C>, Receiver<C>)
where
    C: Container + CreateBounded,
{
    let state = Arc::new(State::new(ZQueue::bounded(capacity)));
    let sender = Sender::new(state.clone());
    let receiver = Receiver::new(state);
    (sender, receiver)
}

pub fn unbounded<C>() -> (Sender<C>, Receiver<C>)
where
    C: Container + CreateUnbounded,
{
    let state = Arc::new(State::new(ZQueue::unbounded()));
    let sender = Sender::new(state.clone());
    let receiver = Receiver::new(state);
    (sender, receiver)
}

pub enum SendError<T> {
    Full(T),
    Disconnected(T),
}

impl<T> core::fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Full(_) => f.debug_struct("Full").finish(),
            Self::Disconnected(_) => f.debug_struct("Disconnected").finish(),
        }
    }
}

impl<T> core::fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Full(_) => f.write_str("Channel is full"),
            Self::Disconnected(_) => f.write_str("Channel is disconnected"),
        }
    }
}

#[cfg(feature = "std")]
impl<T> std::error::Error for SendError<T> {}

pub enum RecvError {
    Disconnected,
}

impl core::fmt::Debug for RecvError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Disconnected => f.debug_struct("Disconnected").finish(),
        }
    }
}

impl core::fmt::Display for RecvError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Disconnected => f.write_str("Channel is disconnected"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for RecvError {}

#[derive(Debug)]
struct State<C> {
    queue: ZQueue<C>,
    sender_count: CachePadded<AtomicUsize>,
    receiver_count: CachePadded<AtomicUsize>,
}

impl<C> State<C> {
    fn new(queue: ZQueue<C>) -> Self {
        Self {
            queue,
            sender_count: CachePadded::new(AtomicUsize::new(0)),
            receiver_count: CachePadded::new(AtomicUsize::new(0)),
        }
    }
}

#[cfg(not(feature = "triomphe"))]
use alloc::sync::Arc;
use core::num::NonZeroUsize;
use core::sync::atomic::Ordering;

use event_listener::Listener;
#[cfg(feature = "triomphe")]
use triomphe::Arc;

use super::{SendError, State};
use crate::container::Container;

#[derive(Debug, Clone)]
pub struct Sender<C> {
    state: Arc<State<C>>,
}

impl<C: Container> Sender<C> {
    pub(super) fn new(state: Arc<State<C>>) -> Self {
        Self { state }
    }

    pub fn len(&self) -> usize {
        self.state.queue.len()
    }

    pub fn capacity(&self) -> Option<NonZeroUsize> {
        self.state.queue.capacity()
    }

    pub fn is_empty(&self) -> bool {
        self.state.queue.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.state.queue.is_full()
    }

    pub fn sender_count(&self) -> usize {
        self.state.sender_count.load(Ordering::Acquire)
    }

    pub fn receiver_count(&self) -> usize {
        self.state.receiver_count.load(Ordering::Acquire)
    }
    
    pub fn is_disconnected(&self) -> bool {
        self.state.receiver_count.load(Ordering::Acquire) == 0
    }
    
    pub fn try_send(&self, item: C::Item) -> Result<(), SendError<C::Item>> {
        if self.state.receiver_count.load(Ordering::Acquire) == 0 {
            return Err(SendError::Disconnected(item));
        }
        self.state.queue.try_push(item).map_err(SendError::Full)
    }

    pub fn send(&self, mut item: C::Item) -> Result<(), SendError<C::Item>> {
        if !self.state.queue.has_capacity {
            let result = self.try_send(item);
            debug_assert!(
                !matches!(result, Err(SendError::Full(_))),
                "Unbounded container failed to push"
            );
            return result;
        }

        match self.try_send(item) {
            Ok(()) => return Ok(()),
            Err(SendError::Disconnected(v)) => return Err(SendError::Disconnected(v)),
            Err(SendError::Full(v)) => item = v,
        }

        let backoff = crossbeam_utils::Backoff::new();
        loop {
            match self.try_send(item) {
                Ok(()) => return Ok(()),
                Err(SendError::Disconnected(v)) => return Err(SendError::Disconnected(v)),
                Err(SendError::Full(v)) => item = v,
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            event_listener::listener!(self.state.queue.pop_event => listener);
            match self.try_send(item) {
                Ok(()) => return Ok(()),
                Err(SendError::Disconnected(v)) => return Err(SendError::Disconnected(v)),
                Err(SendError::Full(v)) => item = v,
            }

            listener.wait();
        }
    }

    pub async fn send_async(&self, mut item: C::Item) -> Result<(), SendError<C::Item>> {
        if !self.state.queue.has_capacity {
            let result = self.try_send(item);
            debug_assert!(
                !matches!(result, Err(SendError::Full(_))),
                "Unbounded container failed to push"
            );
            return result;
        }

        loop {
            match self.try_send(item) {
                Ok(()) => return Ok(()),
                Err(SendError::Disconnected(v)) => return Err(SendError::Disconnected(v)),
                Err(SendError::Full(v)) => item = v,
            }

            event_listener::listener!(self.state.queue.pop_event => listener);
            match self.try_send(item) {
                Ok(()) => return Ok(()),
                Err(SendError::Disconnected(v)) => return Err(SendError::Disconnected(v)),
                Err(SendError::Full(v)) => item = v,
            }

            listener.await;
        }
    }
}

impl<C> Drop for Sender<C> {
    fn drop(&mut self) {
        self.state.sender_count.fetch_sub(1, Ordering::Release);
        self.state.queue.push_event.notify(usize::MAX);
    }
}

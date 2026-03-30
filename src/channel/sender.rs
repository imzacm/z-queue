#[cfg(not(feature = "triomphe"))]
use alloc::sync::Arc;
use core::num::NonZeroUsize;
use core::sync::atomic::Ordering;

#[cfg(feature = "triomphe")]
use triomphe::Arc;

use super::{SendError, State};
use crate::container::Container;

#[derive(Debug)]
pub struct Sender<C> {
    state: Arc<State<C>>,
}

impl<C: Container> Clone for Sender<C> {
    fn clone(&self) -> Self {
        Self::new(self.state.clone())
    }
}

impl<C: Container> Sender<C> {
    pub(super) fn new(state: Arc<State<C>>) -> Self {
        state.sender_count.fetch_add(1, Ordering::Release);
        Self { state }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.state.queue.len()
    }

    #[inline(always)]
    pub fn capacity(&self) -> Option<NonZeroUsize> {
        self.state.queue.capacity()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.state.queue.is_empty()
    }

    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.state.queue.is_full()
    }

    #[inline(always)]
    pub fn sender_count(&self) -> usize {
        self.state.sender_count.load(Ordering::Acquire)
    }

    #[inline(always)]
    pub fn receiver_count(&self) -> usize {
        self.state.receiver_count.load(Ordering::Acquire)
    }

    #[inline(always)]
    pub fn is_disconnected(&self) -> bool {
        self.receiver_count() == 0
    }

    #[inline(always)]
    pub fn try_send(&self, item: C::Item) -> Result<(), SendError<C::Item>> {
        if self.is_disconnected() {
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

            let listener = self.state.queue.pop_event.listener();
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

            let listener = self.state.queue.pop_event.listener();
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
        // Wake all receivers if this is the last sender.
        if self.state.sender_count.fetch_sub(1, Ordering::Release) == 1 {
            self.state.queue.push_event.notify(usize::MAX);
        }
    }
}

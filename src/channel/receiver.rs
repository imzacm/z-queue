#[cfg(not(feature = "triomphe"))]
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::num::NonZeroUsize;
use core::sync::atomic::Ordering;

use event_listener::Listener;
#[cfg(feature = "triomphe")]
use triomphe::Arc;

use super::{RecvError, State};
use crate::container::Container;

#[derive(Debug)]
pub struct Receiver<C> {
    state: Arc<State<C>>,
}

impl<C> Clone for Receiver<C> {
    fn clone(&self) -> Self {
        Self { state: self.state.clone() }
    }
}

impl<C: Container> Receiver<C> {
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
        self.state.sender_count.load(Ordering::Acquire) == 0
    }

    pub fn try_recv(&self) -> Result<Option<C::Item>, RecvError> {
        if self.state.sender_count.load(Ordering::Acquire) == 0 {
            return Err(RecvError::Disconnected);
        }
        Ok(self.state.queue.try_pop())
    }

    pub fn recv(&self) -> Result<C::Item, RecvError> {
        match self.try_recv() {
            Ok(Some(v)) => return Ok(v),
            Ok(None) => (),
            Err(e) => return Err(e),
        }

        let backoff = crossbeam_utils::Backoff::new();
        loop {
            match self.try_recv() {
                Ok(Some(v)) => return Ok(v),
                Ok(None) => (),
                Err(e) => return Err(e),
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            event_listener::listener!(self.state.queue.push_event => listener);
            match self.try_recv() {
                Ok(Some(v)) => return Ok(v),
                Ok(None) => (),
                Err(e) => return Err(e),
            }

            listener.wait();
        }
    }

    pub async fn recv_async(&self) -> Result<C::Item, RecvError> {
        loop {
            match self.try_recv() {
                Ok(Some(v)) => return Ok(v),
                Ok(None) => (),
                Err(e) => return Err(e),
            }

            event_listener::listener!(self.state.queue.push_event => listener);
            match self.try_recv() {
                Ok(Some(v)) => return Ok(v),
                Ok(None) => (),
                Err(e) => return Err(e),
            }

            listener.await;
        }
    }

    pub fn try_find<F>(&self, find_fn: F) -> Result<Option<C::Item>, RecvError>
    where
        F: FnMut(&C::Item) -> bool,
    {
        if self.state.sender_count.load(Ordering::Acquire) == 0 {
            return Err(RecvError::Disconnected);
        }
        Ok(self.state.queue.try_find(find_fn))
    }

    pub fn find<F>(&self, mut find_fn: F) -> Result<C::Item, RecvError>
    where
        F: FnMut(&C::Item) -> bool,
    {
        match self.try_find(&mut find_fn) {
            Ok(Some(v)) => return Ok(v),
            Ok(None) => (),
            Err(e) => return Err(e),
        }

        let backoff = crossbeam_utils::Backoff::new();
        loop {
            match self.try_find(&mut find_fn) {
                Ok(Some(v)) => return Ok(v),
                Ok(None) => (),
                Err(e) => return Err(e),
            }

            if !backoff.is_completed() {
                backoff.snooze();
                continue;
            }

            event_listener::listener!(self.state.queue.push_event => listener);
            match self.try_find(&mut find_fn) {
                Ok(Some(v)) => return Ok(v),
                Ok(None) => (),
                Err(e) => return Err(e),
            }

            listener.wait();
        }
    }

    pub async fn find_async<F>(&self, mut find_fn: F) -> Result<C::Item, RecvError>
    where
        F: FnMut(&C::Item) -> bool,
    {
        loop {
            match self.try_find(&mut find_fn) {
                Ok(Some(v)) => return Ok(v),
                Ok(None) => (),
                Err(e) => return Err(e),
            }

            event_listener::listener!(self.state.queue.push_event => listener);
            match self.try_find(&mut find_fn) {
                Ok(Some(v)) => return Ok(v),
                Ok(None) => (),
                Err(e) => return Err(e),
            }

            listener.await;
        }
    }

    pub fn retain<F>(&self, retain_fn: F) -> Result<(), RecvError>
    where
        F: FnMut(&C::Item) -> bool,
    {
        if self.state.sender_count.load(Ordering::Acquire) == 0 {
            return Err(RecvError::Disconnected);
        }
        self.state.queue.retain(retain_fn);
        Ok(())
    }

    pub fn retain_into<F>(&self, retain_fn: F, into: &mut Vec<C::Item>) -> Result<(), RecvError>
    where
        F: FnMut(&C::Item) -> bool,
    {
        if self.state.sender_count.load(Ordering::Acquire) == 0 {
            return Err(RecvError::Disconnected);
        }
        self.state.queue.retain_into(retain_fn, into);
        Ok(())
    }

    pub fn visit<F>(&self, visit_fn: F) -> Result<(), RecvError>
    where
        F: FnMut(&C::Item),
    {
        if self.state.sender_count.load(Ordering::Acquire) == 0 {
            return Err(RecvError::Disconnected);
        }
        self.state.queue.visit(visit_fn);
        Ok(())
    }

    #[cfg(feature = "rand")]
    pub fn rand_shuffle<R: rand::Rng>(&self, rng: &mut R) -> Result<(), RecvError> {
        if self.state.sender_count.load(Ordering::Acquire) == 0 {
            return Err(RecvError::Disconnected);
        }
        self.state.queue.rand_shuffle(rng);
        Ok(())
    }

    #[cfg(feature = "fastrand")]
    pub fn fastrand_shuffle(&self) -> Result<(), RecvError> {
        if self.state.sender_count.load(Ordering::Acquire) == 0 {
            return Err(RecvError::Disconnected);
        }
        self.state.queue.fastrand_shuffle();
        Ok(())
    }
}

impl<C> Drop for Receiver<C> {
    fn drop(&mut self) {
        self.state.receiver_count.fetch_sub(1, Ordering::Release);
        self.state.queue.pop_event.notify(usize::MAX);
    }
}

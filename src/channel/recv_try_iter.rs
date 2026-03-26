use super::{Receiver, RecvError};
use crate::container::Container;

#[derive(Debug)]
pub struct RecvTryIter<C> {
    receiver: Receiver<C>,
}

impl<C> RecvTryIter<C> {
    pub(super) fn new(receiver: Receiver<C>) -> Self {
        Self { receiver }
    }

    pub fn receiver(&self) -> &Receiver<C> {
        &self.receiver
    }

    pub fn into_receiver(self) -> Receiver<C> {
        self.receiver
    }
}

impl<C: Container> Iterator for RecvTryIter<C> {
    type Item = C::Item;

    fn next(&mut self) -> Option<Self::Item> {
        match self.receiver.try_recv() {
            Ok(Some(v)) => Some(v),
            Ok(None) | Err(RecvError::Disconnected) => None,
        }
    }
}

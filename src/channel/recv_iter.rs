use super::Receiver;
use crate::container::Container;

#[derive(Debug)]
pub struct RecvIter<C> {
    receiver: Receiver<C>,
}

impl<C> RecvIter<C> {
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

impl<C: Container> Iterator for RecvIter<C> {
    type Item = C::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

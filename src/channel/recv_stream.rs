use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;

use super::{Receiver, RecvError};
use crate::container::Container;
use crate::notify::NotifyListener;

#[ouroboros::self_referencing]
#[derive(Debug)]
struct RecvStreamInner<C: 'static> {
    receiver: Receiver<C>,
    #[borrows(receiver)]
    #[covariant]
    push_listener: Option<NotifyListener<'this>>,
}

#[derive(Debug)]
pub struct RecvStream<C: 'static> {
    inner: RecvStreamInner<C>,
}

impl<C> RecvStream<C> {
    pub(super) fn new(receiver: Receiver<C>) -> Self {
        Self { inner: RecvStreamInner::new(receiver, |_| None) }
    }

    pub fn receiver(&self) -> &Receiver<C> {
        self.inner.borrow_receiver()
    }

    pub fn into_receiver(self) -> Receiver<C> {
        self.inner.into_heads().receiver
    }
}

impl<C: Container> Stream for RecvStream<C> {
    type Item = C::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();

        loop {
            match this.receiver().try_recv() {
                Ok(Some(v)) => {
                    this.inner.with_mut(|inner| *inner.push_listener = None);
                    return Poll::Ready(Some(v));
                }
                Ok(None) => (),
                Err(RecvError::Disconnected) => return Poll::Ready(None),
            }

            let is_pending = this.inner.with_push_listener_mut(|listener| {
                let poll = listener.as_mut().map(|listener| Pin::new(listener).poll(cx));
                match poll {
                    Some(Poll::Ready(_)) => {
                        *listener = None;
                        false
                    }
                    Some(Poll::Pending) => true,
                    None => false,
                }
            });

            if is_pending {
                return Poll::Pending;
            }

            this.inner.with_mut(|inner| {
                let listener = inner.receiver.state.queue.push_event.listener();
                *inner.push_listener = Some(listener);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::StreamExt;
    use tokio::time::sleep;

    use crate::defaults::unbounded;

    #[tokio::test]
    async fn test_recv_stream_lifecycle() {
        let (tx, rx) = unbounded();
        let mut stream = rx.into_stream();

        // --- 1. Test the Fast Path ---
        // Push items before the stream even starts polling.
        // (Assuming your API uses `send_async`. Adjust if it's just `send`).
        tx.send_async(10).await.unwrap();
        tx.send_async(20).await.unwrap();

        assert_eq!(stream.next().await, Some(10), "Stream should yield immediate items");
        assert_eq!(stream.next().await, Some(20), "Stream should yield immediate items");

        // --- 2. Test the Wakeup Logic (Listener) ---
        // Spawn a background task to push items after a delay.
        // This forces `stream.next().await` to return Poll::Pending and go to sleep.
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(50)).await;
            tx_clone.send_async(30).await.unwrap();

            sleep(Duration::from_millis(50)).await;
            tx_clone.send_async(40).await.unwrap();
        });

        // The stream should wake up and yield these as they arrive.
        assert_eq!(stream.next().await, Some(30), "Stream failed to wake up for item 30");
        assert_eq!(stream.next().await, Some(40), "Stream failed to wake up for item 40");

        // --- 3. Test Disconnect ---
        // Drop the original sender. The background task sender is already dropped.
        drop(tx);

        // The queue is empty and disconnected. It MUST return None to terminate the stream.
        assert_eq!(stream.next().await, None, "Stream did not return None on disconnect");

        // Polling after completion should ideally continue to return None.
        assert_eq!(stream.next().await, None, "Stream should safely return None fused");
    }
}

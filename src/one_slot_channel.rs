//! Channel preserving only the last sent value.
use std::{
    convert::Infallible,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use futures::Sink;
use tokio::sync::Notify;

/// Create a channel that preserves only the last value sent to it.
///
/// Unlike `watch`, it doesn't require values to be `Clone`.
///
/// It also does not support closing, on either end. Meaning that reading from
/// receiver end will block indefinitely if the sender end is dropped.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let inner = Arc::new(Inner {
        slot: Mutex::new(None),
        notify: Notify::new(),
    });
    (Sender(inner.clone()), Receiver(inner))
}

struct Inner<T> {
    slot: Mutex<Option<T>>,
    notify: Notify,
}

/// Sender end of the channel.
///
/// Implements `[Sink]`.
pub struct Sender<T>(Arc<Inner<T>>);

impl<T> Sender<T> {
    /// Send a value to the channel.
    pub fn send(&self, v: T) {
        *self.0.slot.lock().expect("Poisoned Mutex") = Some(v);
        self.0.notify.notify_one();
    }
}

impl<T> Sink<T> for &Sender<T> {
    type Error = Infallible;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        self.send(item);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

impl<T> Sink<T> for Sender<T> {
    type Error = Infallible;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        self.send(item);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

/// Receiver end of the channel.
///
/// Reading from receiver once sender has been dropped will block indefinitely.
pub struct Receiver<T>(Arc<Inner<T>>);

impl<T> Receiver<T> {
    /// Receive a value.
    ///
    /// If at the moment of this call the channel contains an unseen value,
    /// it will be returned. Otherwise this will block until new value is
    /// sent.
    pub async fn recv(&self) -> T {
        loop {
            self.0.notify.notified().await;
            let val = self.0.slot.lock().expect("Poisoned Mutex").take();
            if let Some(val) = val {
                break val;
            }
        }
    }
}

use std::{
    convert::Infallible,
    path::Path,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use abort_on_drop::ChildTask;
use futures::{Sink, SinkExt, Stream, StreamExt as _, never::Never};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::mpsc;
use tokio_util::sync::{PollSendError, PollSender};

use super::{Client, RetryInterval};

type MappedPollSink<Request> =
    futures::sink::SinkMapErr<PollSender<Request>, fn(PollSendError<Request>) -> Infallible>;

/// Robust client that will seamlessly reconnect on failures.
pub struct RobustClient<Request, Response> {
    incoming: mpsc::Receiver<Response>,
    outgoing: MappedPollSink<Request>,
    client_loop: ChildTask<Never>,
}

impl<Request, Response> RobustClient<Request, Response>
where
    Response: DeserializeOwned + Send + 'static,
    Request: Serialize + Send + Sync + 'static,
{
    /// Construct a new [RobustClient].
    pub fn new(
        socket_path: &Path,
        retry_interval: impl RetryInterval + Send + Clone + 'static,
    ) -> Self {
        let socket_path = socket_path.to_owned();
        let (incoming_tx, incoming_rx) = mpsc::channel(10);
        let (outgoing_tx, outgoing_rx) = mpsc::channel(10);
        let mut out_stream = tokio_stream::wrappers::ReceiverStream::new(outgoing_rx);
        let client_loop = async move {
            let mut pending = None;
            'outer: loop {
                // split() will break lifetimes, and we want lifetimes...
                let mut client =
                    Client::<Request, Response>::new_retry(&socket_path, retry_interval.clone())
                        .await;
                let mut stream = futures::stream::iter(pending.take())
                    .chain(&mut out_stream)
                    .fuse();
                loop {
                    tokio::select! {
                        // if stream ends, then the sink part got dropped. We
                        // just ignore this branch then. Stream is fused, so
                        // this is safe, if a tad inefficient. Generally, this
                        // won't happen, as recv-only stream is already
                        // implemented on Self.
                        Some(next) = stream.next() => {
                            if let Err(e) = client.send(&next).await {
                                pending = Some(next);
                                tracing::error!(?e);
                                continue 'outer;
                            }
                        },
                        res = client.next() => {
                            let next = match res {
                                Some(Ok(x)) => x,
                                Some(Err(e)) => {
                                    tracing::error!(?e);
                                    continue 'outer;
                                }
                                None => continue 'outer,
                            };
                            if incoming_tx.send(next).await.is_err() {
                                // incoming_tx is closed, but maybe that's by
                                // design; just ignore it.
                                continue;
                            }
                        },
                    }
                }
            }
        };
        Self {
            incoming: incoming_rx,
            outgoing: PollSender::new(outgoing_tx).sink_map_err(|e| {
                unreachable!("Error should be impossible here: {e}");
            }),
            client_loop: ChildTask::from(tokio::spawn(client_loop)),
        }
    }

    /// Produce a [Sink] for requests.
    pub fn sink(&mut self) -> impl Sink<Request, Error = Infallible> + use<'_, Request, Response> {
        &mut self.outgoing
    }

    /// Produce a [Sink] for requests and [Stream] for responses. Borrowing
    /// version. Should be slightly more efficient than [Self::split_owned].
    pub fn split_borrowed(
        &mut self,
    ) -> (
        impl Sink<Request, Error = Infallible> + use<'_, Request, Response>,
        impl Stream<Item = Response> + use<'_, Request, Response>,
    ) {
        (&mut self.outgoing, SplitStream(&mut self.incoming))
    }

    /// Produce a [Sink] for requests and [Stream] for responses. Owned version.
    pub fn split_owned(self) -> (SplitSinkOwned<Request>, SplitStreamOwned<Response>) {
        let task = Arc::new(self.client_loop);
        (
            SplitSinkOwned {
                inner: self.outgoing,
                _task: Arc::clone(&task),
            },
            SplitStreamOwned {
                inner: self.incoming,
                _task: task,
            },
        )
    }
}

impl<Request, Response> Stream for RobustClient<Request, Response>
where
    Response: DeserializeOwned,
{
    type Item = Response;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.incoming.poll_recv(cx)
    }
}

struct SplitStream<'a, T>(&'a mut mpsc::Receiver<T>);

impl<T> Stream for SplitStream<'_, T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}

/// The sink part of the [RobustClient], owned version.
pub struct SplitSinkOwned<Request> {
    inner: MappedPollSink<Request>,
    _task: Arc<ChildTask<Never>>,
}

impl<Request> SplitSinkOwned<Request> {
    /// A convenient function to send message once, using only shared reference,
    /// unlike [SinkExt::send].
    #[expect(clippy::missing_panics_doc)]
    pub async fn send_ref(&self, msg: Request)
    where
        Request: Send,
    {
        self.inner
            .get_ref()
            .get_ref()
            .expect("Must be open by construction")
            .send(msg)
            .await
            .expect("Receiver part isn't dropped until self");
    }
}

impl<Request> Sink<Request> for SplitSinkOwned<Request>
where
    MappedPollSink<Request>: Sink<Request, Error = Infallible> + Unpin,
{
    type Error = Infallible;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready_unpin(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Request) -> Result<(), Self::Error> {
        self.inner.start_send_unpin(item)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_flush_unpin(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_close_unpin(cx)
    }
}

/// The stream part of the [RobustClient], owned version.
pub struct SplitStreamOwned<Response> {
    inner: mpsc::Receiver<Response>,
    _task: Arc<ChildTask<Never>>,
}

impl<Response> Stream for SplitStreamOwned<Response> {
    type Item = Response;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.poll_recv(cx)
    }
}

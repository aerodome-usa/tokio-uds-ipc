use std::{
    convert::Infallible,
    path::Path,
    pin::Pin,
    task::{Context, Poll},
};

use abort_on_drop::ChildTask;
use futures::{Sink, SinkExt, Stream, StreamExt as _, never::Never};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::mpsc;
use tokio_util::sync::{PollSendError, PollSender};

pub mod borrowed;
pub mod owned;

use crate::one_slot_channel;

use super::{Client, RetryInterval};

type MappedPollSink<Request> =
    futures::sink::SinkMapErr<PollSender<Request>, fn(PollSendError<Request>) -> Infallible>;

/// Robust client that will seamlessly reconnect on failures.
pub struct RobustClient<Request, Response> {
    incoming: mpsc::Receiver<Response>,
    /// Sink for requests with resilient delivery.
    outgoing_resl: MappedPollSink<Request>,
    /// Channel for constantly sent requests.
    outgoing_live: one_slot_channel::Sender<Request>,
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
        let (outgoing_resl_tx, outgoing_resl_rx) = mpsc::channel(10);
        let (outgoing_live_tx, outgoing_live_rx) = one_slot_channel::channel();
        let mut out_resl_stream = tokio_stream::wrappers::ReceiverStream::new(outgoing_resl_rx);
        let client_loop = async move {
            let mut pending = None;
            'outer: loop {
                // split() will break lifetimes, and we want lifetimes...
                let mut client =
                    Client::<Request, Response>::new_retry(&socket_path, retry_interval.clone())
                        .await;
                let mut out_resl_stream = futures::stream::iter(pending.take())
                    .chain(&mut out_resl_stream)
                    .fuse();
                loop {
                    tokio::select! {
                        // if stream ends, then the sink part got dropped. We
                        // just ignore this branch then. Stream is fused, so
                        // this is safe, if a tad inefficient. Generally, this
                        // won't happen, as recv-only stream is already
                        // implemented on Self.
                        Some(next) = out_resl_stream.next() => {
                            if let Err(e) = client.send(&next).await {
                                pending = Some(next);
                                tracing::error!(?e);
                                continue 'outer;
                            }
                        },
                        next = outgoing_live_rx.recv() => {
                            if let Err(e) = client.send(&next).await {
                                tracing::error!(?e);
                                continue 'outer;
                            }
                        }
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
            outgoing_resl: PollSender::new(outgoing_resl_tx).sink_map_err(|e| {
                unreachable!("Error should be impossible here: {e}");
            }),
            outgoing_live: outgoing_live_tx,
            client_loop: ChildTask::from(tokio::spawn(client_loop)),
        }
    }

    /// Produce a [Sink] for requests.
    pub fn sink(&mut self) -> impl Sink<Request, Error = Infallible> + use<'_, Request, Response> {
        &mut self.outgoing_resl
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

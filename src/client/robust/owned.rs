//! Helper types carrying parts of [super::RobustClient], owned version.
use std::{
    convert::Infallible,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use abort_on_drop::ChildTask;
use futures::{Sink, SinkExt, Stream, never::Never};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::mpsc;

use crate::{one_slot_channel, utils::sink_via_unpin};

use super::MappedPollSink;

/// The stream part of the [super::RobustClient].
pub struct SplitStream<Response> {
    inner: mpsc::Receiver<Response>,
    _task: Arc<ChildTask<Never>>,
}

impl<Response> Stream for SplitStream<Response> {
    type Item = Response;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.poll_recv(cx)
    }
}

/// The combination of [super::RobustClient]'s sinks.
pub struct SplitSink<Request> {
    /// Sink with resilient delivery.
    pub resilient: SplitResilientSink<Request>,
    /// Sink for cases when requests are sent constantly, and only the recent
    /// request is relevant. Requests will not be preserved on reconnection.
    pub live: SplitLiveSink<Request>,
}

/// The resilient sink part of the [super::RobustClient].
pub struct SplitResilientSink<Request> {
    inner: MappedPollSink<Request>,
    _task: Arc<ChildTask<Never>>,
}

impl<Request> SplitResilientSink<Request> {
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

impl<Request> Sink<Request> for SplitResilientSink<Request>
where
    MappedPollSink<Request>: Sink<Request, Error = Infallible> + Unpin,
{
    type Error = Infallible;
    sink_via_unpin!(inner);
}

/// The live sink part of the [super::RobustClient].
pub struct SplitLiveSink<Request> {
    inner: one_slot_channel::Sender<Request>,
    _task: Arc<ChildTask<Never>>,
}

impl<Request> Sink<Request> for SplitLiveSink<Request> {
    type Error = Infallible;
    sink_via_unpin!(inner);
}

impl<Request, Response> super::RobustClient<Request, Response>
where
    Response: DeserializeOwned + Send + 'static,
    Request: Serialize + Send + Sync + 'static,
{
    /// Produce a [SplitSink] containing several [Sink]s for requests and
    /// [Stream] for responses. Owned version.
    pub fn split_owned(self) -> (SplitSink<Request>, SplitStream<Response>) {
        let task = Arc::new(self.client_loop);
        (
            SplitSink {
                resilient: SplitResilientSink {
                    inner: self.outgoing_resl,
                    _task: Arc::clone(&task),
                },
                live: SplitLiveSink {
                    inner: self.outgoing_live,
                    _task: Arc::clone(&task),
                },
            },
            SplitStream {
                inner: self.incoming,
                _task: task,
            },
        )
    }
}

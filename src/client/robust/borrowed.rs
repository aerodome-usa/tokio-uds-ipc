//! Helper types carrying parts of [super::RobustClient], borrowed version.
use std::{
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{Sink, SinkExt, Stream};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::mpsc;

use crate::{one_slot_channel, utils::sink_via_unpin};

use super::MappedPollSink;

/// The stream part of the [super::RobustClient].
pub struct SplitStream<'a, T>(&'a mut mpsc::Receiver<T>);

impl<T> Stream for SplitStream<'_, T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}

/// The combination of [super::RobustClient]'s sinks.
pub struct SplitSink<'a, Request> {
    /// Sink with resilient delivery.
    pub resilient: ResilientSplitSink<'a, Request>,
    /// Sink for cases when requests are sent constantly, and only the recent
    /// request is relevant. Requests will not be preserved on reconnection.
    pub live: LiveSplitSink<'a, Request>,
}

/// The resilient sink part of the [super::RobustClient].
pub struct ResilientSplitSink<'a, Request>(&'a mut MappedPollSink<Request>);

/// The live sink part of the [super::RobustClient].
pub struct LiveSplitSink<'a, Request>(&'a one_slot_channel::Sender<Request>);

impl<Request> Sink<Request> for ResilientSplitSink<'_, Request>
where
    MappedPollSink<Request>: Sink<Request, Error = Infallible> + Unpin,
{
    type Error = Infallible;
    sink_via_unpin!(0);
}

impl<Request> Sink<Request> for LiveSplitSink<'_, Request> {
    type Error = Infallible;
    sink_via_unpin!(0);
}

impl<Request, Response> super::RobustClient<Request, Response>
where
    Response: DeserializeOwned + Send + 'static,
    Request: Serialize + Send + Sync + 'static,
{
    /// Produce a [SplitSink] containing several [Sink]s for requests and
    /// [Stream] for responses. Borrowed version.
    pub fn split_borrowed(
        &mut self,
    ) -> (
        SplitSink<Request>,
        impl Stream<Item = Response> + use<'_, Request, Response>,
    ) {
        (
            SplitSink {
                resilient: ResilientSplitSink(&mut self.outgoing_resl),
                live: LiveSplitSink(&self.outgoing_live),
            },
            SplitStream(&mut self.incoming),
        )
    }
}

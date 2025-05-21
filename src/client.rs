//! Client for a UDS service.

mod robust;

use std::{ops::RangeInclusive, path::Path, time::Duration};

use futures::{Sink, SinkExt, Stream, StreamExt};
use serde::{Serialize, de::DeserializeOwned};
use tokio::net::UnixStream;
use tokio_util::codec::Framed;

use crate::comm::{CodecError, PostcardCodec};

pub use self::robust::{RobustClient, borrowed, owned};

/// Generic unix domain socket client. Takes a `Request` and `Response` types.
/// The former is something sent to the server, the latter is what the server
/// sends back.
pub struct Client<Request, Response> {
    pub(crate) inner: Framed<UnixStream, PostcardCodec<Request, Response>>,
}

/// Trait for [Client::new_retry] interval argument.
pub trait RetryInterval {
    fn into_range(self) -> RangeInclusive<Duration>;
}

impl RetryInterval for Duration {
    fn into_range(self) -> RangeInclusive<Duration> {
        self..=self
    }
}

impl RetryInterval for RangeInclusive<Duration> {
    fn into_range(self) -> RangeInclusive<Duration> {
        self
    }
}

// NB: the trait is not actually exported from the crate by design. Rustc
// complains (as in, gives a complie-time error) if a pub type synonym
// references an associated type in a private trait. It doesn't if the trait is
// pub but not exported though, which is what we use here.
pub trait ServerTypeTrait {
    type ServerType;
    type RobustType;
}

impl<Req, Resp> ServerTypeTrait for Client<Req, Resp> {
    type ServerType = crate::Server<Req, Resp>;
    type RobustType = crate::RobustClient<Req, Resp>;
}

/// Get a [crate::Server] type for the given [Client].
pub type ServerType<Client> = <Client as ServerTypeTrait>::ServerType;

/// Get a [crate::RobustClient] type for the given [Client].
pub type RobustType<Client> = <Client as ServerTypeTrait>::RobustType;

impl<Request, Response> Client<Request, Response> {
    /// Make a new [Client] using the socket at the given path.
    pub async fn new(socket_path: &Path) -> Result<Self, std::io::Error> {
        #[expect(path_statements)]
        {
            type_traits::Assert::<Request>::IS_NOT_ZST;
            type_traits::Assert::<Response>::IS_NOT_ZST;
        }

        let sock = UnixStream::connect(socket_path).await?;
        Ok(Self {
            inner: Framed::new(sock, PostcardCodec::new()),
        })
    }

    /// As [Self::new], but will retry on error until success. Will use
    /// exponential backoff. If `retry_interval` is just [Duration], then backoff is
    /// disabled.
    ///
    /// The delay between retries starts at
    /// `min_backoff`, and doubles on each retry, until it reaches `max_backoff`. If
    /// the inner tasks at any point runs for longer than `current_backoff`, the
    /// backoff will reset back to `min_backoff`.
    pub async fn new_retry(socket_path: &Path, retry_interval: impl RetryInterval) -> Self {
        crate::utils::retry_with_backoff(retry_interval.into_range(), || async {
            Self::new(socket_path)
                .await
                .inspect_err(|err| {
                    tracing::error!(
                        ?err,
                        "Failed to connect to {}",
                        socket_path.to_string_lossy()
                    );
                })
                .ok()
        })
        .await
    }

    /// Construct a new [RobustClient].
    pub fn new_robust(
        socket_path: &Path,
        retry_interval: impl RetryInterval + Send + Clone + 'static,
    ) -> RobustClient<Request, Response>
    where
        Response: DeserializeOwned + Send + 'static,
        Request: Serialize + Send + Sync + 'static,
    {
        RobustClient::new(socket_path, retry_interval)
    }
}

impl<Request, Response> Stream for Client<Request, Response>
where
    Response: DeserializeOwned,
{
    type Item = Result<Response, CodecError>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.poll_next_unpin(cx)
    }
}

impl<T, Request, Response> Sink<T> for Client<Request, Response>
where
    T: std::borrow::Borrow<Request>,
    Request: Serialize,
{
    type Error = CodecError;

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready_unpin(cx)
    }

    fn start_send(mut self: std::pin::Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        self.inner.start_send_unpin(item.borrow())
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_flush_unpin(cx)
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_close_unpin(cx)
    }
}

//! Server implementation.

use std::{marker::PhantomData, os::unix::fs::PermissionsExt, path::Path, sync::Arc};

use abort_on_drop::ChildTask;
use futures::{Stream, StreamExt, never::Never};
use tokio::{
    net::{UnixListener, UnixStream},
    task::JoinError,
};
use tokio_util::codec::Framed;

use crate::comm::{CodecError, PostcardCodec};

/// Server errors.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Found existing socket, likely from the previous run, but failed to remove it.
    #[error("Found existing socket, likely from the previous run, but failed to remove it: {0}")]
    RmStaleSock(std::io::Error),

    /// Could not create parent directories for the socket path.
    #[error("Could not create parent directories for the socket path: {0}")]
    MkParentDir(std::io::Error),

    ///Could not bind to the socket
    #[error("Could not bind to the socket {socket}: {source}")]
    SocketBind {
        /// Underlying IO error
        source: std::io::Error,
        /// Socket we tried to bind to.
        socket: String,
    },

    /// Could not set permissions on the socket
    #[error("Could not set permissions on the socket {socket}: {source}")]
    SetSocketPermissions {
        /// Underlying IO error
        source: std::io::Error,
        /// Socket we tried to set permissions on.
        socket: String,
    },

    /// Error in codec.
    #[error("Codec error: {0}")]
    Codec(#[from] CodecError),
}

/// Server handle. If dropped, the server will shut down.
pub struct Server<Request, Response> {
    task: ChildTask<Never>,
    num_clients: tokio::sync::watch::Receiver<usize>,
    _phantom: PhantomData<fn(Request) -> Response>,
}

impl<Request, Response> Server<Request, Response> {
    /// Get a [tokio::sync::watch::Receiver] for the current number of clients.
    pub fn num_clients(&self) -> tokio::sync::watch::Receiver<usize> {
        self.num_clients.clone()
    }

    /// Wait for the server to exit; It will only exit if it panics though,
    /// which it normally shouldn't.
    pub async fn join(self) -> JoinError {
        let Err(e) = self.task.await;
        e
    }
}

/// A concrete type we pass a the request stream to the handler callback. Could
/// be a `BoxStream`, but for now it's more specific to save on `dyn` overhead.
type RequestStreamInner<Request, Response> = futures::stream::FilterMap<
    futures::stream::SplitStream<crate::Client<Response, Request>>,
    std::future::Ready<Option<Request>>,
    fn(Result<Request, CodecError>) -> std::future::Ready<Option<Request>>,
>;

/// An opaque wrapper for the [serve] callback argument. A [Stream] of requests.
pub struct RequestStream<Request, Response>(RequestStreamInner<Request, Response>);

impl<Request, Response> Stream for RequestStream<Request, Response>
where
    RequestStreamInner<Request, Response>: Stream<Item = Request>,
{
    type Item = Request;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.0.poll_next_unpin(cx)
    }
}

/// Run the server at the given socket path. Accepts the following arguments:
///
/// - Path to the socket to create.
/// - A function to create a response stream from a request stream.
///
/// The last argument is a function, and not just a stream and sink, because it
/// needs to be created on-demand per-client. We can't share the same
/// request/response streams between multiple clients.
pub fn serve<Request, Response, S>(
    socket: &Path,
    handler: impl Fn(RequestStream<Request, Response>) -> S + Send + Sync + 'static,
) -> Result<Server<Request, Response>, Error>
where
    Request: for<'de> serde::Deserialize<'de> + Send,
    Response: serde::Serialize + Send,
    S: Stream<Item = Response> + Send,
{
    #[expect(path_statements)]
    {
        type_traits::Assert::<Request>::IS_NOT_ZST;
        type_traits::Assert::<Response>::IS_NOT_ZST;
    }

    if socket.exists() {
        std::fs::remove_file(socket).map_err(Error::RmStaleSock)?;
    }
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent).map_err(Error::MkParentDir)?;
    }
    let listener = UnixListener::bind(socket).map_err(|e| Error::SocketBind {
        source: e,
        socket: socket.display().to_string(),
    })?;

    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o666)).map_err(|e| {
        Error::SocketBind {
            source: e,
            socket: socket.display().to_string(),
        }
    })?;

    let (num_clients_tx, num_clients_rx) = tokio::sync::watch::channel(0);

    Ok(Server {
        task: ChildTask::from(tokio::spawn(async move {
            serve_loop(listener, handler, num_clients_tx).await
        })),
        num_clients: num_clients_rx,
        _phantom: PhantomData,
    })
}

async fn serve_loop<Request, Response, S>(
    listener: UnixListener,
    handler: impl Fn(RequestStream<Request, Response>) -> S + Send + Sync + 'static,
    num_clients: tokio::sync::watch::Sender<usize>,
) -> !
where
    Request: for<'de> serde::Deserialize<'de> + Send,
    Response: serde::Serialize + Send,
    S: Stream<Item = Response> + Send,
{
    let handler = Arc::new(handler);
    let cancel = tokio_util::sync::CancellationToken::new();
    let cancelled = cancel.child_token();
    let _cancel_guard = cancel.drop_guard();
    loop {
        let (client, addr) = match listener.accept().await {
            Ok(sock) => sock,
            Err(err) => {
                tracing::error!(?err, "Client connection failed");
                continue;
            }
        };
        tracing::info!("Client {addr:?} connected");
        tokio::spawn({
            let handler = handler.clone();
            let num_clients = num_clients.clone();
            let cancelled = cancelled.clone();
            async move {
                num_clients.send_modify(|x| {
                    *x = x.saturating_add(1);
                });
                let res = tokio::select! {
                    res = serve_connection(client, handler.as_ref()) => res,
                    _ = cancelled.cancelled() => Ok(()),
                };
                if let Err(err) = res {
                    tracing::error!(?err, "Client connection completed with an error");
                }
                num_clients.send_modify(|x| {
                    *x = x.saturating_sub(1);
                });
            }
        });
    }
}

async fn serve_connection<Request, Response, S>(
    sock: UnixStream,
    handler: impl Fn(RequestStream<Request, Response>) -> S,
) -> Result<(), Error>
where
    Request: for<'de> serde::Deserialize<'de>,
    Response: serde::Serialize,
    S: Stream<Item = Response>,
{
    let io = crate::Client {
        inner: Framed::new(sock, PostcardCodec::<Response, Request>::new()),
    };

    let (sink, stream) = io.split();

    fn filter_errs<Request>(
        msg: Result<Request, CodecError>,
    ) -> std::future::Ready<Option<Request>> {
        std::future::ready(match msg {
            Ok(msg) => Some(msg),
            Err(err) => {
                tracing::error!(?err, "Decoding an incoming message failed");
                None
            }
        })
    }

    let request_stream = stream.filter_map(
        filter_errs::<Request>
            as fn(Result<Request, CodecError>) -> std::future::Ready<Option<Request>>,
    );

    handler(RequestStream(request_stream))
        .map(Ok)
        .forward(sink)
        .await?;

    Ok(())
}

/// Make a callback for [serve] that constructs the response stream
/// independently of handling the request stream. Both request and response
/// streams are still polled in the same task, and the request stream will be
/// exhausted before the next poll of the response stream. Avoid blocking in
/// request handler, as it will block the response stream as well.
pub fn independent_handlers<Request, Response, S, F>(
    request_handler: F,
    response_stream: impl Fn() -> S,
) -> impl Fn(RequestStream<Request, Response>) -> IndependentHandlers<Request, Response, S, F>
where
    F: Fn(Request),
    S: Stream<Item = Response>,
    RequestStream<Request, Response>: Stream<Item = Request>,
{
    let request_handler = Arc::new(request_handler);
    move |req: RequestStream<Request, Response>| IndependentHandlers {
        handler: Arc::clone(&request_handler),
        req: req.fuse(),
        resp: response_stream(),
    }
}

/// Return stream type for [independent_handlers].
pub struct IndependentHandlers<Request, Response, S, F> {
    handler: Arc<F>,
    req: futures::stream::Fuse<RequestStream<Request, Response>>,
    resp: S,
}

impl<Request, Response, S, F> Stream for IndependentHandlers<Request, Response, S, F>
where
    RequestStream<Request, Response>: Stream<Item = Request>,
    S: Stream<Item = Response> + Unpin,
    F: Fn(Request) + Unpin,
{
    type Item = Response;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        while let std::task::Poll::Ready(Some(msg)) = self.as_mut().req.poll_next_unpin(cx) {
            (self.handler)(msg);
        }
        self.resp.poll_next_unpin(cx)
    }
}

#[cfg(test)]
mod test {
    use crate::client::Client;

    use super::*;

    use std::{path::Path, time::Duration};

    use futures::SinkExt;
    use serde::{Deserialize, Serialize};
    use tokio::sync::broadcast;

    #[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
    struct Resp {
        foo: String,
        bar: String,
    }

    #[derive(Deserialize, Serialize)]
    struct Req {
        foo: String,
        bar: String,
    }

    #[tokio::test]
    async fn test_comm() {
        let sock_path = Path::new("/tmp/uds-client-server.sock");

        let (tx, rx) = broadcast::channel(100);

        let handler = independent_handlers(
            move |msg: Req| {
                let _ = tx.send(Resp {
                    foo: msg.bar,
                    bar: msg.foo,
                });
            },
            move || {
                tokio_stream::wrappers::BroadcastStream::new(rx.resubscribe())
                    .filter_map(|x| std::future::ready(x.ok()))
            },
        );

        let _server = serve::<Req, Resp, _>(sock_path, handler).unwrap();

        let mut client =
            Client::<Req, Resp>::new_retry(sock_path, Duration::from_millis(100)).await;

        client
            .send(Req {
                foo: "test1".to_owned(),
                bar: "test2".to_owned(),
            })
            .await
            .unwrap();

        assert_eq!(
            client.next().await.unwrap().unwrap(),
            Resp {
                foo: "test2".to_owned(),
                bar: "test1".to_owned(),
            }
        );

        client
            .send(Req {
                foo: "test3".to_owned(),
                bar: "test4".to_owned(),
            })
            .await
            .unwrap();

        assert_eq!(
            client.next().await.unwrap().unwrap(),
            Resp {
                foo: "test4".to_owned(),
                bar: "test3".to_owned(),
            }
        );

        client
            .send(Req {
                foo: "test5".to_owned(),
                bar: "test6".to_owned(),
            })
            .await
            .unwrap();

        client
            .send(Req {
                foo: "test7".to_owned(),
                bar: "test8".to_owned(),
            })
            .await
            .unwrap();

        assert_eq!(
            client.next().await.unwrap().unwrap(),
            Resp {
                foo: "test6".to_owned(),
                bar: "test5".to_owned(),
            }
        );

        assert_eq!(
            client.next().await.unwrap().unwrap(),
            Resp {
                foo: "test8".to_owned(),
                bar: "test7".to_owned(),
            }
        );
    }
}

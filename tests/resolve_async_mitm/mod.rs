//! Asynchronous MITM fixture used by resolve timeout integration tests.

use super::utils::ShutdownHandle;
use bytes::Bytes;
use futures::future::BoxFuture;
use http_body_util::Full;
use hyper::{Request, Response, body::Incoming, service::service_fn};
use hyper_util::{rt::TokioExecutor, server::conn::auto};
use std::{
    io::ErrorKind,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{net::TcpListener, sync::oneshot};

/// Shared asynchronous handler type invoked for each incoming request.
type AsyncHandler = Arc<
    Mutex<Box<dyn FnMut(Request<Incoming>) -> BoxFuture<'static, Response<Full<Bytes>>> + Send>>,
>;

/// Start an HTTP server forwarding requests to an asynchronous shared handler.
///
/// # Errors
///
/// Returns an error if the server fails to bind to a local port.
///
/// # Panics
///
/// Panics if the default response cannot be constructed.
#[expect(
    clippy::integer_division_remainder_used,
    reason = "tokio::select! uses % internally"
)]
pub(super) async fn start_async_mitm()
-> Result<(SocketAddr, AsyncHandler, ShutdownHandle), std::io::Error> {
    let handler: AsyncHandler = Arc::new(Mutex::new(Box::new(|_req| {
        Box::pin(async {
            Response::builder()
                .status(404)
                .body(Full::from("No handler"))
                .expect("failed to create default response")
        })
    })));
    let handler_clone = handler.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (tx, mut rx) = oneshot::channel();

    let join = tokio::spawn(async move {
        let builder = auto::Builder::new(TokioExecutor::new());
        loop {
            tokio::select! {
                res = listener.accept() => match res {
                    Ok((stream, _)) => {
                        let io = hyper_util::rt::TokioIo::new(stream);
                        let handler = handler_clone.clone();
                        let service = service_fn(move |request: Request<Incoming>| {
                            let handler = handler.clone();
                            async move {
                                let response = {
                                    let mut handler = handler.lock().expect("lock handler in service");
                                    (handler)(request)
                                };
                                Ok::<_, std::convert::Infallible>(response.await)
                            }
                        });
                        let builder = builder.clone();
                        tokio::spawn(async move {
                            let _ = builder.serve_connection(io, service).await;
                        });
                    }
                    Err(error) => {
                        eprintln!("accept error: {error}");
                        match error.kind() {
                            ErrorKind::ConnectionAborted
                            | ErrorKind::ConnectionReset
                            | ErrorKind::Interrupted
                            | ErrorKind::WouldBlock => {}
                            _ => break,
                        }
                    }
                },
                _ = &mut rx => break,
            }
        }
    });

    Ok((addr, handler, ShutdownHandle::new(join, tx)))
}

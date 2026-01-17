//! Test utilities for setting up MITM proxy servers.
//!
//! This module provides helper functions for end-to-end testing that require
//! intercepting HTTP requests with customisable response handlers.

use assert_cmd::prelude::*;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode, body::Incoming, service::service_fn};
use hyper_util::{rt::TokioExecutor, server::conn::auto};
use std::io::ErrorKind;
use std::{
    collections::VecDeque,
    net::SocketAddr,
    process::Command,
    sync::{Arc, Mutex},
};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

/// Shared handler type invoked for each incoming request.
pub type Handler = Arc<Mutex<Box<dyn FnMut(&Request<Incoming>) -> Response<Full<Bytes>> + Send>>>;

/// Shared handler type for capturing request bodies.
pub type CaptureHandler =
    Arc<Mutex<Box<dyn Fn(&Request<Bytes>) -> Response<Full<Bytes>> + Send + Sync>>>;

/// Handle returned by [`start_mitm`] for shutting down the server.
pub struct ShutdownHandle {
    join: JoinHandle<()>,
    stop: oneshot::Sender<()>,
}

impl ShutdownHandle {
    /// Signal the server to stop and await shutdown.
    pub async fn shutdown(self) {
        let _ = self.stop.send(());
        let _ = self.join.await;
    }
}

/// Start an HTTP server forwarding requests to a shared handler.
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
pub async fn start_mitm() -> Result<(SocketAddr, Handler, ShutdownHandle), std::io::Error> {
    let handler: Handler = Arc::new(Mutex::new(Box::new(|_req| {
        Response::builder()
            .status(404)
            .body(Full::from("No handler"))
            .expect("failed to create default response")
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
                        let h = handler_clone.clone();
                        let service = service_fn(move |req: Request<Incoming>| {
                            let mut f = h.lock().expect("lock handler in service");
                            let resp = (f)(&req);
                            async move { Ok::<_, std::convert::Infallible>(resp) }
                        });
                        let builder = builder.clone();
                        tokio::spawn(async move {
                            let conn = builder.serve_connection(io, service);
                            let _ = conn.await;
                        });
                    }
                    Err(e) => {
                        eprintln!("accept error: {e}");
                        match e.kind() {
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

    Ok((addr, handler, ShutdownHandle { join, stop: tx }))
}

/// Start an HTTP server forwarding requests to a shared handler while capturing request bodies for assertions.
///
/// # Errors
///
/// Returns an error if the server fails to bind to a local port.
///
/// # Panics
///
/// Panics if the default response cannot be constructed.
#[allow(dead_code, clippy::type_complexity, reason = "used only in some tests")]
#[expect(
    clippy::integer_division_remainder_used,
    reason = "tokio::select! uses % internally"
)]
pub async fn start_mitm_capture()
-> Result<(SocketAddr, CaptureHandler, ShutdownHandle), std::io::Error> {
    use http_body_util::BodyExt;
    let handler: CaptureHandler = Arc::new(Mutex::new(Box::new(|_req| {
        Response::builder()
            .status(404)
            .body(Full::from(Bytes::from_static(b"No handler")))
            .expect("failed to create default response")
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
                        let h = handler_clone.clone();
                        let service = service_fn(move |req: Request<Incoming>| {
                            let h = h.clone();
                            async move {
                                let (parts, body) = req.into_parts();
                                let bytes = body.collect().await.unwrap_or_default().to_bytes();
                                let req2 = Request::from_parts(parts, bytes);
                                let f = h.lock().expect("lock handler in service");
                                let resp = (f)(&req2);
                                Ok::<_, std::convert::Infallible>(resp)
                            }
                        });
                        let builder = builder.clone();
                        tokio::spawn(async move {
                            let _ = builder.serve_connection(io, service).await;
                        });
                    }
                    Err(e) => {
                        eprintln!("accept error: {e}");
                        match e.kind() {
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

    Ok((addr, handler, ShutdownHandle { join, stop: tx }))
}
/// Create a `vk` command configured for testing.
///
/// The command points at the MITM server for both GraphQL and REST requests and disables colour output to make
/// assertions deterministic.
#[allow(
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    reason = "helper for integration tests"
)]
pub fn vk_cmd(addr: SocketAddr) -> Command {
    let mut cmd = Command::cargo_bin("vk").expect("binary");
    cmd.env("GITHUB_GRAPHQL_URL", format!("http://{addr}/graphql"))
        .env("GITHUB_API_URL", format!("http://{addr}"))
        .env("GITHUB_TOKEN", "dummy")
        .env("NO_COLOR", "1")
        .env("CLICOLOR_FORCE", "0");
    cmd
}

/// Configure handler to respond with bodies sequentially.
///
/// # Panics
///
/// Panics if a response body is missing or if building the response fails.
#[allow(dead_code, reason = "helper used in some tests only")]
pub fn set_sequential_responder(handler: &Handler, bodies: impl Into<Vec<String>>) {
    let responses = Arc::new(Mutex::new(VecDeque::from(bodies.into())));
    let responses_clone = Arc::clone(&responses);
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = responses_clone
            .lock()
            .expect("lock responses")
            .pop_front()
            .expect("response");
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("build response")
    });
}

/// Configure capture handler with request body assertions.
///
/// This handler uses `start_mitm_capture` to access request bodies and allows
/// asserting GraphQL variables in requests for end-to-end wiring verification.
///
/// # Panics
///
/// Panics if a response body is missing or if building the response fails.
#[allow(dead_code, reason = "helper used in some tests only")]
pub fn set_sequential_responder_with_assert<F>(
    handler: &CaptureHandler,
    bodies: impl Into<Vec<String>>,
    assert_fn: F,
) where
    F: Fn(&serde_json::Value) + Send + Sync + 'static,
{
    let responses = Arc::new(Mutex::new(VecDeque::from(bodies.into())));
    let responses_clone = Arc::clone(&responses);
    let assert_fn = Arc::new(assert_fn);
    *handler.lock().expect("lock handler") = Box::new(move |req: &Request<Bytes>| {
        let body_bytes = req.body();
        let json = serde_json::from_slice::<serde_json::Value>(body_bytes)
            .expect("invalid JSON request body");
        assert_fn(&json);
        let body = responses_clone
            .lock()
            .expect("lock responses")
            .pop_front()
            .expect("response");
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("build response")
    });
}

const _: fn(SocketAddr) -> Command = vk_cmd;

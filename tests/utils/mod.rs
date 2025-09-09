//! Test utilities for setting up MITM proxy servers.
//!
//! This module provides helper functions for end-to-end testing that require
//! intercepting HTTP requests with customisable response handlers.

use assert_cmd::prelude::*;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, body::Incoming, service::service_fn};
use hyper_util::{rt::TokioExecutor, server::conn::auto};
use std::io::ErrorKind;
use std::{
    net::SocketAddr,
    process::Command,
    sync::{Arc, Mutex},
};
use tokio::{net::TcpListener, sync::oneshot, task::JoinHandle};

/// Shared handler type invoked for each incoming request.
pub type Handler = Arc<Mutex<Box<dyn FnMut(&Request<Incoming>) -> Response<Full<Bytes>> + Send>>>;

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

const _: fn(SocketAddr) -> Command = vk_cmd;

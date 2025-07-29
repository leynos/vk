use hyper::{
    Body, Request, Response, Server,
    service::{make_service_fn, service_fn},
};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::task::JoinHandle;

pub type Handler = Arc<Mutex<Box<dyn FnMut(&Request<Body>) -> Response<Body> + Send>>>;

/// Start an HTTP server forwarding requests to a shared handler.
///
/// # Panics
///
/// Panics if the server fails to bind to a local port.
#[must_use]
pub fn start_mitm() -> (SocketAddr, Handler, JoinHandle<()>) {
    let handler: Handler = Arc::new(Mutex::new(Box::new(|_req| {
        Response::builder()
            .status(404)
            .body(Body::from("No handler"))
            .expect("default response")
    })));
    let handler_clone = handler.clone();

    let make_svc = make_service_fn(move |_conn| {
        let h = handler_clone.clone();
        async move {
            Ok::<_, std::convert::Infallible>(service_fn(move |req: Request<Body>| {
                let mut f = h.lock().expect("lock handler");
                let resp = (f)(&req);
                async move { Ok::<_, std::convert::Infallible>(resp) }
            }))
        }
    });

    let server = Server::bind(&"127.0.0.1:0".parse().expect("parse addr")).serve(make_svc);
    let addr = server.local_addr();
    let handle = tokio::spawn(async move {
        let _ = server.await;
    });
    (addr, handler, handle)
}

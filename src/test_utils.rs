//! Common test utilities.
//!
//! Provides helpers for manipulating environment variables and spinning up a
//! stub GraphQL server for integration tests.

use crate::api::{GraphQLClient, RetryConfig};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicUsize, Ordering},
};
use third_wheel::hyper::{
    Body, Request, Response, Server, StatusCode,
    service::{make_service_fn, service_fn},
};
use tokio::{task::JoinHandle, time::Duration};

/// Stub client and server handle for HTTP tests.
pub struct TestClient {
    /// Client targeting the stub server.
    pub client: GraphQLClient,
    /// Handle for stopping the server task.
    pub join: JoinHandle<()>,
    /// Count of HTTP requests received.
    pub hits: Arc<AtomicUsize>,
}

/// Start a stub HTTP server returning each body in `responses` sequentially.
///
/// Returns a [`GraphQLClient`] targeting the server and a [`JoinHandle`] for
/// the server task.
///
/// # Examples
///
/// ```no_run
/// use vk::test_utils::start_server;
///
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// let body = String::from("{}");
/// let server = start_server(vec![body]);
/// server.join.abort();
/// let _ = server.join.await;
/// # });
/// ```
pub fn start_server(responses: Vec<String>) -> TestClient {
    let responses = Arc::new(responses);
    let counter = Arc::new(AtomicUsize::new(0));
    let svc_counter = Arc::clone(&counter);
    let svc = make_service_fn(move |_conn| {
        let responses = Arc::clone(&responses);
        let counter = Arc::clone(&svc_counter);
        async move {
            Ok::<_, std::convert::Infallible>(service_fn(move |_req: Request<Body>| {
                let idx = counter.fetch_add(1, Ordering::SeqCst);
                let body = responses
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| "{}".to_string());
                async move {
                    Ok::<_, std::convert::Infallible>(
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", "application/json")
                            .body(Body::from(body))
                            .expect("response"),
                    )
                }
            }))
        }
    });
    let server = Server::bind(&"127.0.0.1:0".parse().expect("parse addr")).serve(svc);
    let addr = server.local_addr();
    let join = tokio::spawn(async move {
        let _ = server.await;
    });
    let retry = RetryConfig {
        base_delay: Duration::from_millis(1),
        ..RetryConfig::default()
    };
    let client = GraphQLClient::with_endpoint_retry("token", format!("http://{addr}"), None, retry)
        .expect("create client");
    TestClient {
        client,
        join,
        hits: counter,
    }
}

/// Set an environment variable for testing.
///
/// Environment manipulation is process-wide and therefore not thread-safe.
/// A global mutex serialises modifications so parallel tests do not race.
///
/// # Examples
///
/// ```ignore
/// use vk::test_utils::{set_var, remove_var};
///
/// set_var("MY_VAR", "1");
/// assert_eq!(std::env::var("MY_VAR"), Ok("1".into()));
/// remove_var("MY_VAR");
/// ```
pub fn set_var<K: AsRef<std::ffi::OsStr>, V: AsRef<std::ffi::OsStr>>(key: K, value: V) {
    let _guard = env_lock();
    // SAFETY: The global mutex serialises access to the environment, making the
    // unsynchronised standard library calls safe for our tests.
    unsafe { std::env::set_var(key, value) };
}

/// Remove an environment variable set during testing.
///
/// The global mutex serialises modifications so parallel tests do not race.
pub fn remove_var<K: AsRef<std::ffi::OsStr>>(key: K) {
    let _guard = env_lock();
    // SAFETY: The global mutex serialises access to the environment, making the
    // unsynchronised standard library calls safe for our tests.
    unsafe { std::env::remove_var(key) };
}

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock")
}

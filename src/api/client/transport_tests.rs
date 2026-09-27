//! Loopback behavioural tests for the GraphQL HTTP transport.

use super::MAX_RESPONSE_BODY_BYTES;
use crate::{
    VkError,
    api::{GraphQLClient, RetryConfig},
};
use bytes::Bytes;
use futures::future::join_all;
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    future::Future,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use third_wheel::hyper::{
    Body, Request, Response, Server, StatusCode,
    body::to_bytes,
    service::{make_service_fn, service_fn},
};
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{Duration, sleep},
};
/// Start a loopback GraphQL stub and return its address and task handle.
pub(super) fn start_loopback_server<F, Fut>(handler: F) -> (SocketAddr, JoinHandle<()>)
where
    F: Fn(Request<Body>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Response<Body>, Infallible>> + Send + 'static,
{
    let handler = Arc::new(handler);
    let service = make_service_fn(move |_connection| {
        let handler = Arc::clone(&handler);
        async move {
            Ok::<_, Infallible>(service_fn(move |request| {
                let handler = Arc::clone(&handler);
                async move { handler(request).await }
            }))
        }
    });
    let server =
        Server::bind(&"127.0.0.1:0".parse().expect("parse loopback address")).serve(service);
    let address = server.local_addr();
    let task = tokio::spawn(async move {
        let _ = server.await;
    });
    (address, task)
}
/// Stop a loopback stub after the client has completed its assertion path.
pub(super) async fn stop_loopback_server(task: JoinHandle<()>) {
    task.abort();
    let _ = task.await;
}
/// Build deterministic retry settings for one loopback scenario.
pub(super) fn loopback_retry(timeout: Duration) -> RetryConfig {
    RetryConfig {
        attempts: 0,
        base_delay: Duration::from_millis(1),
        request_timeout: timeout,
        jitter: false,
    }
}
/// Assert that a low-level transport result keeps the supplied diagnostics.
fn assert_request_context(error: &VkError, expected_fragments: &[&str]) {
    match error {
        VkError::RequestContext { .. } => {}
        other => panic!("unexpected error: {other:?}"),
    }
    let diagnostic = error.to_string();
    for fragment in expected_fragments {
        assert!(diagnostic.contains(fragment), "{diagnostic}");
    }
}
/// Execute one named request through the client shared by concurrent callers.
async fn run_concurrent_query(
    client: Arc<GraphQLClient>,
    operation: &'static str,
) -> (&'static str, Value) {
    let query = format!("query {operation} {{ viewer {{ login }} }}");
    let payload = json!({"query": query, "variables": {}, "operationName": operation});
    let response = client
        .run_payload(&payload, operation)
        .await
        .expect("execute concurrent query");
    (operation, response)
}
/// Assert each operation contributes one complete request/response transcript pair.
fn assert_transcript_pairs(transcript: &str, operations: &[&str]) {
    let entries: Vec<Value> = transcript
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse transcript line"))
        .collect();
    assert_eq!(entries.len(), operations.len());
    for operation in operations {
        let matching_entries = entries
            .iter()
            .filter(|entry| {
                entry.get("operation").and_then(Value::as_str) == Some(*operation)
                    && entry
                        .get("request")
                        .and_then(|request| request.get("operationName"))
                        .and_then(Value::as_str)
                        == Some(*operation)
                    && entry
                        .get("response")
                        .and_then(Value::as_str)
                        .is_some_and(|body| body.contains(operation))
            })
            .count();
        assert_eq!(
            matching_entries, 1,
            "missing transcript pair for {operation}"
        );
    }
}
#[tokio::test]
async fn transport_maps_refused_loopback_connections_to_request_context() {
    let address = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("reserve loopback port");
        let address = listener.local_addr().expect("read reserved address");
        drop(listener);
        address
    };
    let client = GraphQLClient::with_endpoint_retry(
        "token",
        format!("http://{address}"),
        None,
        loopback_retry(Duration::from_millis(100)),
    )
    .expect("build GraphQL client");
    let payload = json!({"query": "query RefusedConnection { viewer { login } }", "variables": {}, "operationName": "RefusedConnection"});
    let error = client
        .run_payload::<Value>(&payload, "RefusedConnection")
        .await
        .expect_err("refused connection fails");
    assert_request_context(&error, &[]);
}
#[tokio::test]
async fn transport_preserves_non_success_status_and_body_snippet() {
    let (address, server_task) = start_loopback_server(|_request| async {
        Ok::<_, Infallible>(
            Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from("upstream is unavailable"))
                .expect("build failure response"),
        )
    });
    let client = GraphQLClient::with_endpoint_retry(
        "token",
        format!("http://{address}"),
        None,
        loopback_retry(Duration::from_secs(1)),
    )
    .expect("build GraphQL client");

    let payload = json!({"query": "query NonSuccess { viewer { login } }", "variables": {}, "operationName": "NonSuccess"});
    let error = client
        .run_payload::<Value>(&payload, "NonSuccess")
        .await
        .expect_err("non-success response fails");
    stop_loopback_server(server_task).await;

    assert_request_context(&error, &["status 502", "upstream is unavailable"]);
}
#[tokio::test]
async fn transport_times_out_before_receiving_response_headers() {
    let (address, server_task) = start_loopback_server(|_request| async {
        sleep(Duration::from_millis(100)).await;
        Ok::<_, Infallible>(Response::new(Body::from("{}")))
    });
    let client = GraphQLClient::with_endpoint_retry(
        "token",
        format!("http://{address}"),
        None,
        loopback_retry(Duration::from_millis(20)),
    )
    .expect("build GraphQL client");

    let payload = json!({"query": "query HeaderTimeout { viewer { login } }", "variables": {}, "operationName": "HeaderTimeout"});
    let error = client
        .run_payload::<Value>(&payload, "HeaderTimeout")
        .await
        .expect_err("header wait times out");
    stop_loopback_server(server_task).await;

    assert_request_context(&error, &["HeaderTimeout", "request timed out after"]);
}
#[tokio::test]
async fn transport_rejects_response_bodies_over_the_limit() {
    let oversized_body = vec![b'x'; MAX_RESPONSE_BODY_BYTES + 1];
    let (address, server_task) = start_loopback_server(move |_request| {
        let oversized_body = oversized_body.clone();
        async move {
            Ok::<_, Infallible>(
                Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(Body::from(oversized_body))
                    .expect("build oversized response"),
            )
        }
    });
    let client = GraphQLClient::with_endpoint_retry(
        "token",
        format!("http://{address}"),
        None,
        loopback_retry(Duration::from_secs(1)),
    )
    .expect("build GraphQL client");

    let payload = json!({"query": "query OversizedBody { viewer { login } }", "variables": {}, "operationName": "OversizedBody"});
    let error = client
        .run_payload::<Value>(&payload, "OversizedBody")
        .await
        .expect_err("oversized response fails");
    stop_loopback_server(server_task).await;

    assert_request_context(&error, &["OversizedBody", "status 503"]);
}
#[tokio::test]
async fn transport_reports_non_timeout_response_body_read_failures() {
    let (body_sender, body) = Body::channel();
    let body_sender = Arc::new(Mutex::new(Some(body_sender)));
    let body = Arc::new(Mutex::new(Some(body)));
    let (sender_task_sender, sender_task_receiver) = oneshot::channel();
    let sender_task_sender = Arc::new(Mutex::new(Some(sender_task_sender)));
    let (address, server_task) = start_loopback_server(move |_request| {
        let body_sender = Arc::clone(&body_sender);
        let body = Arc::clone(&body);
        let sender_task_sender = Arc::clone(&sender_task_sender);
        async move {
            let mut body_sender = body_sender
                .lock()
                .expect("lock body sender")
                .take()
                .expect("send one response body");
            let sender_task = tokio::spawn(async move {
                body_sender
                    .send_data(Bytes::from_static(b"{\"data\":"))
                    .await
                    .expect("send initial response body data");
                body_sender.abort();
            });
            sender_task_sender
                .lock()
                .expect("lock sender-task channel")
                .take()
                .expect("send one sender task")
                .send(sender_task)
                .expect("receive sender task");
            Ok::<_, Infallible>(
                Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(
                        body.lock()
                            .expect("lock response body")
                            .take()
                            .expect("send one body"),
                    )
                    .expect("build fallible response"),
            )
        }
    });
    let client = GraphQLClient::with_endpoint_retry(
        "token",
        format!("http://{address}"),
        None,
        loopback_retry(Duration::from_secs(1)),
    )
    .expect("build GraphQL client");

    let payload = json!({"query": "query BrokenBody { viewer { login } }", "variables": {}, "operationName": "BrokenBody"});
    let error = client
        .run_payload::<Value>(&payload, "BrokenBody")
        .await
        .expect_err("body read failure fails the request");
    sender_task_receiver
        .await
        .expect("receive sender task")
        .await
        .expect("complete sender task");
    stop_loopback_server(server_task).await;

    assert_request_context(&error, &["BrokenBody", "status 503"]);
    assert!(
        !error.to_string().contains("request timed out after"),
        "{error}"
    );
}
#[tokio::test]
async fn pooled_transport_keeps_concurrent_responses_and_transcripts_isolated() {
    let (address, server_task) = start_loopback_server(|request| async move {
        let payload: Value = serde_json::from_slice(
            &to_bytes(request.into_body())
                .await
                .expect("read request body"),
        )
        .expect("parse GraphQL request");
        let operation = payload
            .get("operationName")
            .and_then(Value::as_str)
            .expect("request operation name")
            .to_string();
        Ok::<_, Infallible>(Response::new(Body::from(
            json!({"data": {"operation": operation}}).to_string(),
        )))
    });
    let transcript_directory = tempfile::tempdir().expect("create transcript directory");
    let transcript_path = transcript_directory.path().join("graphql.jsonl");
    let client = Arc::new(
        GraphQLClient::with_endpoint_retry(
            "token",
            format!("http://{address}"),
            Some(transcript_path.clone()),
            loopback_retry(Duration::from_secs(1)),
        )
        .expect("build GraphQL client"),
    );
    let operations = [
        "ConcurrentAlpha",
        "ConcurrentBeta",
        "ConcurrentGamma",
        "ConcurrentDelta",
    ];
    let results = join_all(
        operations
            .into_iter()
            .map(|operation| run_concurrent_query(Arc::clone(&client), operation)),
    )
    .await;
    let transcript = std::fs::read_to_string(&transcript_path).expect("read transcript");
    stop_loopback_server(server_task).await;

    for (operation, response) in results {
        assert_eq!(response, json!({"operation": operation}));
    }
    assert_transcript_pairs(&transcript, &operations);
}

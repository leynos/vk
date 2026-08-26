//! Loopback behavioural tests for the GraphQL HTTP transport.

use super::{MAX_RESPONSE_BODY_BYTES, response_body_exceeds_limit};
use crate::{
    VkError,
    api::{GraphQLClient, RetryConfig},
};
use futures::future::join_all;
use proptest::prelude::*;
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    future::Future,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use third_wheel::hyper::{
    Body, Method, Request, Response, Server, StatusCode,
    body::to_bytes,
    service::{make_service_fn, service_fn},
};
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{Duration, sleep},
};

/// Start a loopback GraphQL stub and return its address and task handle.
fn start_loopback_server<F, Fut>(handler: F) -> (SocketAddr, JoinHandle<()>)
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
async fn stop_loopback_server(task: JoinHandle<()>) {
    task.abort();
    let _ = task.await;
}

/// Build deterministic retry settings for one loopback scenario.
fn loopback_retry(timeout: Duration) -> RetryConfig {
    RetryConfig {
        attempts: 0,
        base_delay: Duration::from_millis(1),
        request_timeout: timeout,
        jitter: false,
    }
}

#[derive(Debug)]
struct CapturedRequest {
    method: Method,
    path: String,
    content_type: Option<String>,
    user_agent: Option<String>,
    accept: Option<String>,
    authorization: Option<String>,
    payload: Value,
}

impl CapturedRequest {
    /// Read a loopback request into the fields asserted by the contract test.
    async fn from_request(request: Request<Body>) -> Self {
        let (parts, body) = request.into_parts();
        let payload = serde_json::from_slice(&to_bytes(body).await.expect("read request body"))
            .expect("parse request JSON");
        Self {
            method: parts.method,
            path: parts.uri.path().to_string(),
            content_type: parts
                .headers
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
            user_agent: parts
                .headers
                .get("user-agent")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
            accept: parts
                .headers
                .get("accept")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
            authorization: parts
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string),
            payload,
        }
    }
}

/// Assert the bounded request shape sent to a loopback endpoint override.
fn assert_graphql_request_contract(captured: &CapturedRequest) {
    assert_eq!(captured.method, Method::POST);
    assert_eq!(captured.path, "/graphql-test");
    assert_eq!(captured.content_type.as_deref(), Some("application/json"));
    assert_eq!(captured.user_agent.as_deref(), Some("vk"));
    assert_eq!(
        captured.accept.as_deref(),
        Some("application/vnd.github+json")
    );
    assert_eq!(captured.authorization.as_deref(), Some("Bearer test-token"));
    assert_eq!(
        captured.payload,
        json!({
            "query": "query RequestContract($id: ID!) { viewer { login } }",
            "variables": {"id": "42"},
            "operationName": "RequestContract",
        })
    );
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
    let response = client
        .run_query(query.as_str(), json!({}))
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
async fn transport_posts_graphql_contract_to_the_endpoint_override() {
    let (capture_sender, capture_receiver) = oneshot::channel();
    let capture_sender = Arc::new(Mutex::new(Some(capture_sender)));
    let (address, server_task) = start_loopback_server(move |request| {
        let capture_sender = Arc::clone(&capture_sender);
        async move {
            let captured = CapturedRequest::from_request(request).await;
            capture_sender
                .lock()
                .expect("lock capture sender")
                .take()
                .expect("capture one request")
                .send(captured)
                .expect("receive captured request");
            Ok::<_, Infallible>(Response::new(Body::from(
                json!({"data": {"viewer": {"login": "octocat"}}}).to_string(),
            )))
        }
    });
    let endpoint = format!("http://{address}/graphql-test");
    let client = GraphQLClient::with_endpoint_retry(
        "test-token",
        endpoint,
        None,
        loopback_retry(Duration::from_secs(1)),
    )
    .expect("build GraphQL client");

    let result: Value = client
        .run_query(
            "query RequestContract($id: ID!) { viewer { login } }",
            json!({"id": "42"}),
        )
        .await
        .expect("execute GraphQL query");
    let captured = capture_receiver.await.expect("receive capture");
    stop_loopback_server(server_task).await;

    assert_eq!(result, json!({"viewer": {"login": "octocat"}}));
    assert_graphql_request_contract(&captured);
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

    let error = client
        .run_query::<_, Value>("query RefusedConnection { viewer { login } }", json!({}))
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

    let error = client
        .run_query::<_, Value>("query NonSuccess { viewer { login } }", json!({}))
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

    let error = client
        .run_query::<_, Value>("query HeaderTimeout { viewer { login } }", json!({}))
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

    let error = client
        .run_query::<_, Value>("query OversizedBody { viewer { login } }", json!({}))
        .await
        .expect_err("oversized response fails");
    stop_loopback_server(server_task).await;

    assert_request_context(&error, &["OversizedBody", "status 503"]);
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

proptest! {
    #[test]
    fn response_size_boundary_matches_the_configured_limit(size in any::<usize>()) {
        prop_assert_eq!(
            response_body_exceeds_limit(size),
            size > MAX_RESPONSE_BODY_BYTES,
        );
    }
}

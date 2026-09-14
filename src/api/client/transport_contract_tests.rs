//! Loopback request-contract tests for the GraphQL HTTP transport.

use super::tests::{loopback_retry, start_loopback_server, stop_loopback_server};
use crate::api::GraphQLClient;
use serde_json::{Value, json};
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};
use third_wheel::hyper::{Body, Method, Request, Response, body::to_bytes};
use tokio::{sync::oneshot, time::Duration};

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

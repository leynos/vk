//! Tests for the public [`fetch_issue`] entry point.
//!
//! These exercise the whole path end to end against a stub server: the
//! operation document, the request variables, and the semantic errors the
//! public API promises. Wire-adapter details are covered by [`super::wire`].

use super::*;
use crate::test_utils::{TestClient, start_server};
use serde_json::json;

/// Repository used by every case in this module.
fn repo() -> RepoInfo {
    RepoInfo {
        owner: "owner".into(),
        name: "repository".into(),
    }
}

#[tokio::test]
async fn issue_query_sends_its_operation_and_variables() {
    let body = json!({"data": {"repository": {"issue": {
        "title": "Issue title", "body": "Issue body"
    }}}})
    .to_string();
    let TestClient {
        client,
        join,
        requests,
        ..
    } = start_server(vec![body]);

    let issue = fetch_issue(&client, &repo(), 42)
        .await
        .expect("fetch issue");

    assert_eq!(issue.title, "Issue title");
    assert_eq!(issue.body, "Issue body");
    {
        let requests = requests.lock().expect("lock requests");
        let request = requests.first().expect("one request");
        assert_eq!(request.get("operationName"), Some(&json!("IssueQuery")));
        assert_eq!(request.pointer("/variables/owner"), Some(&json!("owner")));
        assert_eq!(
            request.pointer("/variables/name"),
            Some(&json!("repository"))
        );
        assert_eq!(request.pointer("/variables/number"), Some(&json!(42)));
    }
    join.abort();
    let _ = join.await;
}

#[tokio::test]
async fn missing_issue_returns_a_semantic_error() {
    let TestClient { client, join, .. } =
        start_server(vec![json!({"data": {"repository": null}}).to_string()]);

    let result = fetch_issue(&client, &repo(), 42).await;

    assert!(
        matches!(result, Err(VkError::BadResponse(message)) if message.as_ref() == "issue #42 not found")
    );
    join.abort();
    let _ = join.await;
}

#[tokio::test]
async fn missing_issue_node_returns_a_semantic_error() {
    let TestClient { client, join, .. } = start_server(vec![
        json!({
            "data": {"repository": {"issue": null}}
        })
        .to_string(),
    ]);

    let result = fetch_issue(&client, &repo(), 42).await;

    assert!(
        matches!(result, Err(VkError::BadResponse(message)) if message.as_ref() == "issue #42 not found")
    );
    join.abort();
    let _ = join.await;
}

#[tokio::test]
async fn out_of_range_issue_number_does_not_send_a_request() {
    let TestClient {
        client, join, hits, ..
    } = start_server(Vec::new());

    let result = fetch_issue(&client, &repo(), i32::MAX as u64 + 1).await;

    assert!(matches!(result, Err(VkError::InvalidNumber)));
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    join.abort();
    let _ = join.await;
}

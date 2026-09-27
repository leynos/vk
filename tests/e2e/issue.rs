//! End-to-end tests for the `vk issue` GraphQL workflow.

use super::common::*;
use assert_cmd::prelude::*;
use predicates::str::contains;
use serde_json::{Value, json};

/// Verify the typed issue operation and its request variables.
fn assert_issue_request(request: &Value) {
    assert_eq!(
        request.pointer("/operationName"),
        Some(&json!("IssueQuery"))
    );
    assert_eq!(request.pointer("/variables/owner"), Some(&json!("owner")));
    assert_eq!(
        request.pointer("/variables/name"),
        Some(&json!("repository"))
    );
    assert_eq!(request.pointer("/variables/number"), Some(&json!(42)));
}

#[tokio::test]
async fn issue_prints_title_and_body_from_the_typed_operation() {
    let (addr, handler, shutdown) = start_mitm_capture().await.expect("start server");
    set_sequential_responder_with_assert(
        &handler,
        vec![
            json!({
                "data": {"repository": {"issue": {
                    "title": "Issue title",
                    "body": "Issue body"
                }}}
            })
            .to_string(),
        ],
        assert_issue_request,
    );

    tokio::task::spawn_blocking(move || {
        vk_cmd(addr)
            .args(["issue", "https://github.com/owner/repository/issues/42"])
            .assert()
            .success()
            .stdout(contains("Issue title"))
            .stdout(contains("Issue body"));
    })
    .await
    .expect("spawn blocking");
    shutdown.shutdown().await;
}

#[tokio::test]
async fn issue_reports_a_missing_issue_node() {
    let (addr, handler, shutdown) = start_mitm_capture().await.expect("start server");
    set_sequential_responder_with_assert(
        &handler,
        vec![json!({"data": {"repository": {"issue": null}}}).to_string()],
        assert_issue_request,
    );

    tokio::task::spawn_blocking(move || {
        vk_cmd(addr)
            .args(["issue", "https://github.com/owner/repository/issues/42"])
            .assert()
            .failure()
            .stderr(contains("issue #42 not found"));
    })
    .await
    .expect("spawn blocking");
    shutdown.shutdown().await;
}

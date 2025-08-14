//! CLI integration tests for empty-state messages.
//!
//! These tests verify that the `vk pr` command reports a clear message when no
//! review comments remain, both with and without file filters.

use assert_cmd::prelude::*;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode, body::Incoming};
use rstest::rstest;
use serde_json::json;
use std::process::Command;

use predicates::prelude::*;

mod utils;
use utils::start_mitm;

/// Build a closure returning an empty `reviewThreads` payload.
fn create_empty_review_handler()
-> impl Fn(&Request<Incoming>) -> Response<Full<Bytes>> + Send + 'static {
    move |_req| {
        let body = json!({
            "data": {
                "repository": {
                    "pullRequest": {
                        "reviewThreads": {
                            "nodes": [],
                            "pageInfo": { "hasNextPage": false, "endCursor": null }
                        }
                    }
                }
            }
        })
        .to_string();
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("build response")
    }
}

#[rstest]
#[case(
    Vec::new(),
    "========== code review ==========\nNo unresolved comments.\n========== end of code review ==========\n"
)]
#[case(
    vec!["no_such_file.rs"],
    "========== code review ==========\nNo unresolved comments for the specified files.\n========== end of code review ==========\n",
)]
#[tokio::test]
async fn pr_empty_state(
    #[case] extra_args: Vec<&'static str>,
    #[case] expected_output: &'static str,
) {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    *handler.lock().expect("lock handler") = Box::new(create_empty_review_handler());

    let output = expected_output.to_string();

    tokio::task::spawn_blocking(move || {
        let mut cmd = Command::cargo_bin("vk").expect("binary");
        cmd.env("GITHUB_GRAPHQL_URL", format!("http://{addr}"))
            .env("GITHUB_TOKEN", "dummy")
            .args(["pr", "https://github.com/leynos/shared-actions/pull/42"]);

        for arg in extra_args {
            cmd.arg(arg);
        }

        cmd.assert().success().stdout(output);
    })
    .await
    .expect("spawn blocking");

    shutdown.shutdown().await;
}

#[tokio::test]
async fn pr_outputs_banner_when_threads_present() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let threads_body = json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [{
                "id": "t1",
                "isResolved": false,
                "comments": {
                    "nodes": [{
                        "body": "Looks good",
                        "diffHunk": "@@ -1 +1 @@\n-old\n+new\n",
                        "originalPosition": null,
                        "position": null,
                        "path": "file.rs",
                        "url": "http://example.com",
                        "author": {"login": "alice"}
                    }],
                    "pageInfo": {"hasNextPage": false, "endCursor": null}
                }
            }],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}}
    })
    .to_string();
    let reviews_body = json!({
        "data": {"repository": {"pullRequest": {"reviews": {
            "nodes": [],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}}
    })
    .to_string();
    let mut responses = vec![threads_body, reviews_body].into_iter();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = responses.next().expect("response");
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("build response")
    });

    tokio::task::spawn_blocking(move || {
        let mut cmd = Command::cargo_bin("vk").expect("binary");
        cmd.env("GITHUB_GRAPHQL_URL", format!("http://{addr}"))
            .env("GITHUB_TOKEN", "dummy")
            .args(["pr", "https://github.com/leynos/shared-actions/pull/42"]);
        cmd.assert().success().stdout(
            predicate::str::starts_with("========== code review ==========\n")
                .and(predicate::str::contains("Looks good")),
        );
    })
    .await
    .expect("spawn blocking");

    shutdown.shutdown().await;
}

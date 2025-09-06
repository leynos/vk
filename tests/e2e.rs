//! End-to-end tests for GraphQL error diagnostics.
//!
//! These tests verify that enhanced error reporting works correctly when
//! GraphQL responses contain missing nodes, using mock HTTPS servers to
//! simulate real-world scenarios.
//!
//! Each test spawns a [`third-wheel`](https://crates.io/crates/third-wheel)
//! Man-in-the-Middle proxy that intercepts outbound GitHub requests. This
//! proxy serves canned responses from `tests/fixtures` so the suite runs in a
//! fully hermetic and deterministic manner.

use assert_cmd::cargo::cargo_bin;
use assert_cmd::prelude::*;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;
mod utils;
use hyper::{Response, StatusCode};
use utils::start_mitm;

fn load_transcript(path: &str) -> Vec<String> {
    let data = fs::read_to_string(path).expect("read transcript");
    data.lines()
        .map(|line| {
            let v: Value = serde_json::from_str(line).expect("valid json line");
            v.get("response")
                .and_then(|r| r.as_str())
                .unwrap_or("{}")
                .to_owned()
        })
        .collect()
}

/// Build a default empty `comments` payload.
fn empty_comments_fallback() -> String {
    serde_json::json!({
        "data": {"node": {"comments": {
            "nodes": [],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}
    })
    .to_string()
}

#[tokio::test]
#[ignore = "requires recorded network transcript"]
async fn e2e_pr_42() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let mut responses = load_transcript("tests/fixtures/pr42.json").into_iter();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = responses.next().unwrap_or_else(empty_comments_fallback);
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(http_body_util::Full::from(body))
            .expect("build response")
    });

    Command::cargo_bin("vk")
        .expect("binary executable")
        .env("GITHUB_GRAPHQL_URL", format!("http://{addr}"))
        .env("GITHUB_TOKEN", "dummy")
        .args(["pr", "https://github.com/leynos/shared-actions/pull/42"])
        .assert()
        .success()
        .stdout(contains("end of code review"));
    shutdown.shutdown().await;
}
#[tokio::test]
async fn e2e_missing_nodes_reports_path() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = serde_json::json!({
            "data": {
                "repository": {
                    "pullRequest": {
                        "reviewThreads": {
                            "pageInfo": { "hasNextPage": false, "endCursor": null }
                        },
                        "reviews": {
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
            .body(http_body_util::Full::from(body))
            .expect("build response")
    });

    tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            Command::cargo_bin("vk")
                .expect("binary executable")
                .env("GITHUB_GRAPHQL_URL", format!("http://{addr}"))
                .env("GITHUB_TOKEN", "dummy")
                .args(["pr", "https://github.com/leynos/cmd-mox/pull/25"])
                .assert()
                .failure()
                .stderr(contains("repository.pullRequest.reviewThreads"))
                .stderr(contains("snippet:"));
        }),
    )
    .await
    .expect("command timed out")
    .expect("spawn blocking");
    shutdown.shutdown().await;
}

#[tokio::test]
async fn pr_discussion_reference_fetches_single_thread() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let threads_body = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [{
                "id": "t1",
                "isResolved": false,
                "comments": {
                    "nodes": [
                        { "body": "first", "diffHunk": "@@ -1 +1 @@\n-old\n+new\n", "originalPosition": null, "position": null, "path": "file.rs", "url": "https://github.com/o/r/pull/1#discussion_r1", "author": null },
                        { "body": "second", "diffHunk": "@@ -1 +1 @@\n-old\n+new\n", "originalPosition": null, "position": null, "path": "file.rs", "url": "https://github.com/o/r/pull/1#discussion_r2", "author": null }
                    ],
                    "pageInfo": { "hasNextPage": false, "endCursor": null }
                }
            }],
            "pageInfo": { "hasNextPage": false, "endCursor": null }
        }}}}
    }).to_string();
    let reviews_body = include_str!("fixtures/reviews_empty.json").to_string();
    let mut responses = vec![threads_body, reviews_body].into_iter();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = responses.next().expect("response");
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(http_body_util::Full::from(body))
            .expect("build response")
    });

    tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            Command::cargo_bin("vk")
                .expect("binary executable")
                .env("GITHUB_GRAPHQL_URL", format!("http://{addr}"))
                .env("GITHUB_TOKEN", "dummy")
                .args(["pr", "https://github.com/o/r/pull/1#discussion_r2"])
                .assert()
                .success()
                .stdout(contains("second"))
                .stdout(contains("first").not());
        }),
    )
    .await
    .expect("command timed out")
    .expect("spawn blocking");
    shutdown.shutdown().await;
}

#[tokio::test]
async fn pr_exits_cleanly_on_broken_pipe() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let mut responses = load_transcript("tests/fixtures/pr42.json").into_iter();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = responses.next().unwrap_or_else(empty_comments_fallback);
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(http_body_util::Full::from(body))
            .expect("build response")
    });

    let vk_bin = cargo_bin("vk");
    let status = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::task::spawn_blocking(move || {
            let mut child = Command::new(&vk_bin)
                .env("GITHUB_GRAPHQL_URL", format!("http://{addr}"))
                .env("GITHUB_TOKEN", "dummy")
                .args(["pr", "https://github.com/leynos/shared-actions/pull/42"])
                .stdout(Stdio::piped())
                .spawn()
                .expect("spawn vk");
            let stdout = child.stdout.take().expect("take stdout");
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            drop(reader);
            child.wait().expect("wait vk")
        }),
    )
    .await
    .expect("command timed out")
    .expect("spawn blocking");
    assert!(status.success());
    shutdown.shutdown().await;
}

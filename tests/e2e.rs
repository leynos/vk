//! End-to-end tests validate the `vk` binary using its public interface.
//! Each test spawns a [`third-wheel`](https://crates.io/crates/third-wheel)
//! Man-in-the-Middle proxy that intercepts outbound GitHub requests. This
//! proxy serves canned responses from `tests/fixtures` so the suite runs in a
//! fully hermetic and deterministic manner.

use assert_cmd::Command;
use predicates::str::contains;
use serde_json::Value;
use std::fs;
mod utils;
use third_wheel::hyper::{Body, Response};
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

#[tokio::test]
#[ignore = "requires recorded network transcript"]
async fn e2e_pr_42() {
    let (addr, handler, handle) = start_mitm();
    let mut responses = load_transcript("tests/fixtures/pr42.json").into_iter();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = responses.next().unwrap_or_else(|| "{}".to_string());
        Response::builder()
            .status(200)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .expect("build response")
    });

    Command::cargo_bin("vk")
        .expect("binary")
        .env("GITHUB_GRAPHQL_URL", format!("http://{addr}"))
        .env("GITHUB_TOKEN", "dummy")
        .args(["pr", "https://github.com/leynos/shared-actions/pull/42"])
        .assert()
        .success()
        .stdout(contains("end of code review"));
    handle.abort();
}
#[tokio::test]
async fn e2e_missing_nodes_reports_path() {
    let (addr, handler, handle) = start_mitm();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = serde_json::json!({
            "data": {
                "repository": {
                    "pullRequest": {
                        "reviewThreads": {
                            "pageInfo": { "hasNextPage": false, "endCursor": null }
                        }
                    }
                }
            }
        })
        .to_string();
        Response::builder()
            .status(200)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .expect("build response")
    });

    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::task::spawn_blocking(move || {
            Command::cargo_bin("vk")
                .expect("binary")
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
    handle.abort();
}

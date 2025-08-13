//! CLI integration tests for empty-state messages.
//!
//! These tests verify that the `vk pr` command reports a clear message when no
//! review comments remain, both with and without file filters.

use assert_cmd::prelude::*;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use serde_json::json;
use std::process::Command;

mod utils;
use utils::start_mitm;

#[tokio::test]
async fn pr_empty_state_global() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
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
    });

    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("vk")
            .expect("binary")
            .env("GITHUB_GRAPHQL_URL", format!("http://{addr}"))
            .env("GITHUB_TOKEN", "dummy")
            .args(["pr", "https://github.com/leynos/shared-actions/pull/42"])
            .assert()
            .success()
            .stdout("No unresolved comments.\n========== end of code review ==========\n");
    })
    .await
    .expect("spawn blocking");
    shutdown.shutdown().await;
}

#[tokio::test]
async fn pr_empty_state_filtered() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
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
    });

    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("vk")
            .expect("binary")
            .env("GITHUB_GRAPHQL_URL", format!("http://{addr}"))
            .env("GITHUB_TOKEN", "dummy")
            .args([
                "pr",
                "https://github.com/leynos/shared-actions/pull/42",
                "no_such_file.rs",
            ])
            .assert()
            .success()
            .stdout(
                "No unresolved comments for the specified files.\n========== end of code review ==========\n",
            );
    })
    .await
    .expect("spawn blocking");
    shutdown.shutdown().await;
}

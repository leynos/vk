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
    "No unresolved comments.\n========== end of code review ==========\n"
)]
#[case(
    vec!["no_such_file.rs"],
    "No unresolved comments for the specified files.\n========== end of code review ==========\n",
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

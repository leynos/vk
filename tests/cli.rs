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
use vk::banners::{COMMENTS_BANNER, END_BANNER, START_BANNER};

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
    format!("{START_BANNER}\nNo unresolved comments.\n{END_BANNER}\n"),
)]
#[case(
    vec!["no_such_file.rs"],
    format!(
        "{START_BANNER}\nNo unresolved comments for the specified files.\n{END_BANNER}\n"
    ),
)]
#[tokio::test]
async fn pr_empty_state(#[case] extra_args: Vec<&'static str>, #[case] expected_output: String) {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    *handler.lock().expect("lock handler") = Box::new(create_empty_review_handler());

    let output = expected_output;

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

        let output = cmd.assert().success().get_output().stdout.clone();
        let output_str = String::from_utf8_lossy(&output);

        validate_banner_content(&output_str);
        validate_banner_ordering(&output_str);
    })
    .await
    .expect("spawn blocking");

    shutdown.shutdown().await;
}

/// Confirm banners and comment text are present in the output.
fn validate_banner_content(output: &str) {
    assert!(
        output.starts_with(&format!("{START_BANNER}\n")),
        "Output should start with code review banner",
    );
    assert!(
        output.contains(COMMENTS_BANNER),
        "Output should contain review comments banner",
    );
    let occurrences = output.match_indices(COMMENTS_BANNER).count();
    assert_eq!(
        occurrences, 1,
        "Review comments banner should appear exactly once",
    );
    assert!(
        output.contains("Looks good"),
        "Output should contain 'Looks good'",
    );
    assert!(
        output.contains(END_BANNER),
        "Output should contain end banner",
    );
}

/// Ensure banners and thread output appear in the expected order.
fn validate_banner_ordering(output: &str) {
    let code_idx = output.find(START_BANNER).expect("code review banner");
    let review_idx = output
        .find(COMMENTS_BANNER)
        .expect("review comments banner");
    let thread_idx = output.find("Looks good").expect("thread output");
    let end_idx = output.rfind(END_BANNER).expect("end banner");
    assert!(
        code_idx < review_idx,
        "Review comments banner should appear after code review banner",
    );
    assert!(
        review_idx < thread_idx,
        "Thread output should appear after review comments banner",
    );
    assert!(
        thread_idx < end_idx,
        "End banner should appear after thread output",
    );
}

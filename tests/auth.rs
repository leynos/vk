//! Authentication tests for `GITHUB_TOKEN`.
//!
//! Verifies that vk includes the `Authorization` header when `GITHUB_TOKEN`
//! is present and warns otherwise.

use assert_cmd::prelude::*;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode, body::Incoming, header};
use rstest::rstest;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tokio::time::{Duration, timeout};

mod utils;
use utils::start_mitm;

/// GraphQL body with empty threads and reviews.
const EMPTY_REVIEW_BODY: &str = r#"{
  "data": {
    "repository": {
      "pullRequest": {
        "reviewThreads": {
          "nodes": [],
          "pageInfo": { "hasNextPage": false, "endCursor": null }
        },
        "reviews": {
          "nodes": [],
          "pageInfo": { "hasNextPage": false, "endCursor": null }
        }
      }
    }
  }
}"#;

#[rstest]
#[case(true, Some("Bearer dummy"), false)]
#[case(false, None, true)]
#[tokio::test]
async fn pr_handles_authorisation(
    #[case] has_token: bool,
    #[case] expected_header: Option<&str>,
    #[case] expect_warning: bool,
) {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let captured = Arc::new(Mutex::new(None));
    let captured_clone = captured.clone();
    let body = EMPTY_REVIEW_BODY;
    *handler.lock().expect("lock handler") = Box::new(move |req: &Request<Incoming>| {
        let auth = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        *captured_clone.lock().expect("store header") = auth;
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Full::from(body))
            .expect("build response")
    });

    let addr_str = format!("http://{addr}");
    let task = tokio::task::spawn_blocking(move || {
        let mut cmd = Command::cargo_bin("vk").expect("binary");
        cmd.env("GITHUB_GRAPHQL_URL", addr_str)
            .env("NO_COLOR", "1")
            .env("CLICOLOR_FORCE", "0")
            .env("RUST_LOG", "warn");
        if has_token {
            cmd.env("GITHUB_TOKEN", "dummy");
        } else {
            cmd.env_remove("GITHUB_TOKEN");
        }
        cmd.args(["pr", "https://github.com/leynos/shared-actions/pull/42"]);
        cmd.output().expect("run vk")
    });
    let output = timeout(Duration::from_secs(20), task)
        .await
        .expect("vk timed out")
        .expect("spawn blocking");

    assert!(
        output.status.success(),
        "vk exited with {:?}. Stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    if expect_warning {
        assert!(
            stderr.contains("anonymous API access"),
            "expected warning about anonymous API access"
        );
    } else {
        assert!(
            !stderr.contains("anonymous API access"),
            "unexpected anonymous access warning: {stderr}"
        );
    }
    let header = captured.lock().expect("read header").clone();
    assert_eq!(
        header.as_deref(),
        expected_header,
        "authorisation header mismatch"
    );
    shutdown.shutdown().await;
}

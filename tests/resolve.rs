//! End-to-end tests for the `vk resolve` sub-command.

#![cfg(feature = "unstable-rest-resolve")]

use assert_cmd::prelude::*;
use bytes::Bytes;
use futures::FutureExt as _;
use http_body_util::Full;
use hyper::{
    Request, Response, StatusCode,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
};
use predicates::prelude::*;
use std::sync::{Arc, Mutex};

mod resolve_async_mitm;
mod utils;
use utils::{start_mitm, start_mitm_capture, vk_cmd};

#[path = "resolve/pagination.rs"]
mod pagination;

fn assert_reply_request(req: &Request<Bytes>) {
    assert_eq!(req.body().as_ref(), br#"{"body":"done"}"#);
    assert_eq!(
        req.headers().get(AUTHORIZATION).expect("authorization"),
        "Bearer dummy"
    );
    assert_eq!(
        req.headers().get(ACCEPT).expect("accept"),
        "application/vnd.github+json"
    );
    assert_eq!(
        req.headers()
            .get("x-github-api-version")
            .expect("GitHub API version"),
        "2022-11-28"
    );
    assert_eq!(
        req.headers().get(CONTENT_TYPE).expect("content type"),
        "application/json"
    );
}

async fn run_reply_flow(
    rest_status: StatusCode,
    api_uri_suffix: &str,
) -> (Vec<String>, Vec<u8>, Vec<u8>, std::process::ExitStatus) {
    let (addr, handler, shutdown) = start_mitm_capture().await.expect("start server");
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let clone = Arc::clone(&calls);
    *handler.lock().expect("lock handler") = Box::new(move |req| {
        let mut vec = clone.lock().expect("lock");
        let gql_calls = vec.iter().filter(|c| c.ends_with("/graphql")).count();
        vec.push(format!("{} {}", req.method(), req.uri().path()));
        if req.uri().path().ends_with("/replies") {
            assert_reply_request(req);
        }
        let status = if req.uri().path().ends_with("/replies") {
            rest_status
        } else {
            StatusCode::OK
        };
        let body = if req.uri().path() == "/graphql" {
            if gql_calls == 0 {
                r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"pageInfo":{"endCursor":null,"hasNextPage":false},"nodes":[{"id":"t","comments":{"nodes":[{"fullDatabaseId":"1"}],"pageInfo":{"endCursor":null,"hasNextPage":false}}}]}}}}}"#
            } else {
                r#"{"data":{"resolveReviewThread":{"clientMutationId":null}}}"#
            }
        } else {
            "{}"
        };
        Response::builder()
            .status(status)
            .header(CONTENT_TYPE, "application/json")
            .body(Full::from(body))
            .expect("response")
    });
    let api_uri = format!("http://{addr}{api_uri_suffix}");
    let (stdout, stderr, status) = tokio::task::spawn_blocking(move || {
        let output = vk_cmd(addr)
            .env("GITHUB_API_URL", api_uri)
            .args([
                "resolve",
                "https://github.com/o/r/pull/83#discussion_r1",
                "-m",
                "done",
            ])
            .output()
            .expect("run command");
        (output.stdout, output.stderr, output.status)
    })
    .await
    .expect("spawn blocking");
    shutdown.shutdown().await;
    (calls.lock().expect("lock").clone(), stdout, stderr, status)
}

#[tokio::test]
#[rstest::rstest]
#[case(
    StatusCode::OK,
    true,
    &[
        "POST /repos/o/r/pulls/83/comments/1/replies",
        "POST /graphql",
        "POST /graphql",
    ],
)]
#[case(
    StatusCode::NOT_FOUND,
    true,
    &[
        "POST /repos/o/r/pulls/83/comments/1/replies",
        "POST /graphql",
        "POST /graphql",
    ],
)]
#[case(
    StatusCode::NO_CONTENT,
    true,
    &[
        "POST /repos/o/r/pulls/83/comments/1/replies",
        "POST /graphql",
        "POST /graphql",
    ],
)]
#[case(
    StatusCode::MULTIPLE_CHOICES,
    false,
    &["POST /repos/o/r/pulls/83/comments/1/replies"],
)]
#[case(
    StatusCode::FORBIDDEN,
    false,
    &["POST /repos/o/r/pulls/83/comments/1/replies"],
)]
#[case(
    StatusCode::INTERNAL_SERVER_ERROR,
    false,
    &["POST /repos/o/r/pulls/83/comments/1/replies"],
)]
async fn resolve_flows_reply(
    #[case] rest_status: StatusCode,
    #[case] should_succeed: bool,
    #[case] expected: &'static [&'static str],
) {
    let (calls, stdout, stderr, status) = run_reply_flow(rest_status, "").await;
    let stdout = String::from_utf8_lossy(&stdout);
    let stderr = String::from_utf8_lossy(&stderr);
    let code = rest_status.as_u16().to_string();
    assert!(stdout.trim().is_empty(), "unexpected stdout: {stdout}");
    if should_succeed {
        assert!(status.success(), "status: {status:?}, stderr: {stderr}");
        assert!(stderr.trim().is_empty(), "unexpected stderr: {stderr}");
    } else {
        assert!(!status.success(), "expected failure; got success");
        assert!(
            predicate::str::contains("replies")
                .and(predicate::str::contains(code.as_str()))
                .eval(&stderr),
            "stderr: {stderr}"
        );
    }
    assert_eq!(calls.as_slice(), expected);
}

#[tokio::test]
#[rstest::rstest]
#[case::without_trailing_slash("")]
#[case::with_one_trailing_slash("/")]
#[case::with_multiple_trailing_slashes("///")]
async fn resolve_normalizes_api_uri_trailing_slashes(#[case] api_uri_suffix: &str) {
    let (calls, stdout, stderr, status) = run_reply_flow(StatusCode::OK, api_uri_suffix).await;
    assert!(status.success(), "status: {status:?}, stderr: {stderr:?}");
    assert!(stdout.is_empty(), "unexpected stdout: {stdout:?}");
    assert!(stderr.is_empty(), "unexpected stderr: {stderr:?}");
    assert_eq!(
        calls.as_slice(),
        &[
            "POST /repos/o/r/pulls/83/comments/1/replies",
            "POST /graphql",
            "POST /graphql",
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_reply_honours_total_http_timeout() {
    let (addr, handler, shutdown) = resolve_async_mitm::start_async_mitm()
        .await
        .expect("start server");
    *handler.lock().expect("lock handler") = Box::new(move |req| {
        let is_reply = req.uri().path().ends_with("/replies");
        let body = if req.uri().path() == "/graphql" {
            r#"{"data":{"repository":{"pullRequest":{"reviewComments":{"pageInfo":{"endCursor":null,"hasNextPage":false},"nodes":[{"databaseId":1,"pullRequestReviewThread":{"id":"t"}}]}}}}}"#
        } else {
            "{}"
        };
        async move {
            if is_reply {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/json")
                .body(Full::from(body))
                .expect("response")
        }
        .boxed()
    });
    let output = tokio::task::spawn_blocking(move || {
        vk_cmd(addr)
            .args([
                "--http-timeout",
                "1",
                "resolve",
                "https://github.com/o/r/pull/83#discussion_r1",
                "-m",
                "done",
            ])
            .output()
            .expect("run command")
    })
    .await
    .expect("spawn blocking");
    shutdown.shutdown().await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "expected timeout; stderr: {stderr}"
    );
    assert!(
        predicate::str::contains("post reply to /repos/o/r/pulls/83/comments/1/replies")
            .and(predicate::str::contains("deadline has elapsed"))
            .eval(&stderr),
        "stderr: {stderr}"
    );
}

#[cfg(feature = "unstable-rest-resolve")]
#[tokio::test]
async fn resolve_skips_empty_reply() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let clone = Arc::clone(&calls);
    *handler.lock().expect("lock handler") = Box::new(move |req| {
        let mut vec = clone.lock().expect("lock");
        let gql_calls = vec.iter().filter(|c| c.ends_with("/graphql")).count();
        vec.push(format!("{} {}", req.method(), req.uri().path()));
        let body = if req.uri().path() == "/graphql" {
            if gql_calls == 0 {
                r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"pageInfo":{"endCursor":null,"hasNextPage":false},"nodes":[{"id":"t","comments":{"nodes":[{"fullDatabaseId":"1"}],"pageInfo":{"endCursor":null,"hasNextPage":false}}}]}}}}}"#
            } else {
                r#"{"data":{"resolveReviewThread":{"clientMutationId":null}}}"#
            }
        } else {
            "{}"
        };
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("response")
    });
    tokio::task::spawn_blocking(move || {
        vk_cmd(addr)
            .args([
                "resolve",
                "https://github.com/o/r/pull/83#discussion_r1",
                "-m",
                " ",
            ])
            .assert()
            .success();
    })
    .await
    .expect("spawn blocking");
    shutdown.shutdown().await;
    assert_eq!(
        calls.lock().expect("lock").as_slice(),
        ["POST /graphql", "POST /graphql"],
    );
}

// NOTE: 404 on reply is treated as non-fatal; covered in parameterised tests above.

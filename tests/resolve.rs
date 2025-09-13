//! End-to-end tests for the `vk resolve` sub-command.

#![cfg(feature = "unstable-rest-resolve")]

use assert_cmd::prelude::*;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use predicates::prelude::*;
use std::sync::{Arc, Mutex};

mod utils;
use utils::{start_mitm, vk_cmd};

#[tokio::test]
async fn resolve_flows() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let clone = Arc::clone(&calls);
    *handler.lock().expect("lock handler") = Box::new(move |req| {
        let mut vec = clone.lock().expect("lock");
        let gql_calls = vec.iter().filter(|c| c.ends_with("/graphql")).count();
        vec.push(format!("{} {}", req.method(), req.uri().path()));
        let body = if req.uri().path() == "/graphql" {
            if gql_calls == 0 {
                r#"{"data":{"repository":{"pullRequest":{"reviewComments":{"pageInfo":{"endCursor":null,"hasNextPage":false},"nodes":[{"databaseId":1,"pullRequestReviewThread":{"id":"t"}}]}}}}}"#
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
            .args(["resolve", "https://github.com/o/r/pull/83#discussion_r1"])
            .assert()
            .success()
            .stdout(predicate::str::is_empty())
            .stderr(predicate::str::is_empty());
    })
    .await
    .expect("spawn blocking");
    shutdown.shutdown().await;
    assert_eq!(
        calls.lock().expect("lock").as_slice(),
        ["POST /graphql", "POST /graphql"],
    );
}

#[tokio::test]
async fn resolve_flows_paginates() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let clone = Arc::clone(&calls);
    *handler.lock().expect("lock handler") = Box::new(move |req| {
        let mut vec = clone.lock().expect("lock");
        let gql_calls = vec.iter().filter(|c| c.ends_with("/graphql")).count();
        vec.push(format!("{} {}", req.method(), req.uri().path()));
        let body = if req.uri().path() == "/graphql" {
            match gql_calls {
                0 => {
                    r#"{"data":{"repository":{"pullRequest":{"reviewComments":{"pageInfo":{"endCursor":"c1","hasNextPage":true},"nodes":[{"databaseId":2,"pullRequestReviewThread":{"id":"other"}}]}}}}}"#
                }
                1 => {
                    r#"{"data":{"repository":{"pullRequest":{"reviewComments":{"pageInfo":{"endCursor":null,"hasNextPage":false},"nodes":[{"databaseId":1,"pullRequestReviewThread":{"id":"t"}}]}}}}}"#
                }
                _ => r#"{"data":{"resolveReviewThread":{"clientMutationId":null}}}"#,
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
            .args(["resolve", "https://github.com/o/r/pull/83#discussion_r1"])
            .assert()
            .success()
            .stdout(predicate::str::is_empty())
            .stderr(predicate::str::is_empty());
    })
    .await
    .expect("spawn blocking");
    shutdown.shutdown().await;
    assert_eq!(
        calls.lock().expect("lock").as_slice(),
        ["POST /graphql", "POST /graphql", "POST /graphql"],
    );
}

async fn run_reply_flow(
    rest_status: StatusCode,
) -> (Vec<String>, Vec<u8>, Vec<u8>, std::process::ExitStatus) {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let clone = Arc::clone(&calls);
    *handler.lock().expect("lock handler") = Box::new(move |req| {
        let mut vec = clone.lock().expect("lock");
        let gql_calls = vec.iter().filter(|c| c.ends_with("/graphql")).count();
        vec.push(format!("{} {}", req.method(), req.uri().path()));
        let status = if req.uri().path().ends_with("/replies") {
            rest_status
        } else {
            StatusCode::OK
        };
        let body = if req.uri().path() == "/graphql" {
            if gql_calls == 0 {
                r#"{"data":{"repository":{"pullRequest":{"reviewComments":{"pageInfo":{"endCursor":null,"hasNextPage":false},"nodes":[{"databaseId":1,"pullRequestReviewThread":{"id":"t"}}]}}}}}"#
            } else {
                r#"{"data":{"resolveReviewThread":{"clientMutationId":null}}}"#
            }
        } else {
            "{}"
        };
        Response::builder()
            .status(status)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("response")
    });
    let (stdout, stderr, status) = tokio::task::spawn_blocking(move || {
        let output = vk_cmd(addr)
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
    let (calls, stdout, stderr, status) = run_reply_flow(rest_status).await;
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
                r#"{"data":{"repository":{"pullRequest":{"reviewComments":{"pageInfo":{"endCursor":null,"hasNextPage":false},"nodes":[{"databaseId":1,"pullRequestReviewThread":{"id":"t"}}]}}}}}"#
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

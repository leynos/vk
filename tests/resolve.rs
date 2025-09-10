//! End-to-end tests for the `vk resolve` sub-command.

use assert_cmd::prelude::*;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use predicates::prelude::*;
use std::sync::{Arc, Mutex};

mod utils;
use utils::{start_mitm, vk_cmd};

#[cfg(feature = "unstable-rest-resolve")]
#[tokio::test]
#[rstest::rstest]
#[case(None)]
async fn resolve_flows(#[case] msg: Option<&'static str>) {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let clone = Arc::clone(&calls);
    *handler.lock().expect("lock handler") = Box::new(move |req| {
        let mut vec = clone.lock().expect("lock");
        let gql_calls = vec.iter().filter(|c| c.ends_with("/graphql")).count();
        vec.push(format!("{} {}", req.method(), req.uri().path()));
        let body = if req.uri().path() == "/graphql" {
            if gql_calls == 0 {
                r#"{"data":{"node":{"pullRequestReviewThread":{"id":"t"}}}}"#
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
    let mut args = vec!["resolve", "https://github.com/o/r/pull/83#discussion_r1"];
    if let Some(m) = msg {
        args.extend(["-m", m]);
    }
    tokio::task::spawn_blocking(move || {
        vk_cmd(addr).args(args).assert().success();
    })
    .await
    .expect("spawn blocking");
    shutdown.shutdown().await;
    assert_eq!(
        calls.lock().expect("lock").as_slice(),
        ["POST /graphql", "POST /graphql"],
    );
}

#[cfg(feature = "unstable-rest-resolve")]
#[tokio::test]
async fn resolve_flows_reply() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let clone = Arc::clone(&calls);
    *handler.lock().expect("lock handler") = Box::new(move |req| {
        let mut vec = clone.lock().expect("lock");
        let gql_calls = vec.iter().filter(|c| c.ends_with("/graphql")).count();
        vec.push(format!("{} {}", req.method(), req.uri().path()));
        let body = if req.uri().path() == "/graphql" {
            if gql_calls == 0 {
                r#"{"data":{"node":{"pullRequestReviewThread":{"id":"t"}}}}"#
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
                "done",
            ])
            .assert()
            .success();
    })
    .await
    .expect("spawn blocking");
    shutdown.shutdown().await;
    assert_eq!(
        calls.lock().expect("lock").as_slice(),
        [
            "POST /repos/o/r/pulls/83/comments/1/replies",
            "POST /graphql",
            "POST /graphql",
        ],
    );
}

#[cfg(feature = "unstable-rest-resolve")]
#[tokio::test]
async fn resolve_falls_back_to_rest() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let clone = Arc::clone(&calls);
    *handler.lock().expect("lock handler") = Box::new(move |req| {
        let mut vec = clone.lock().expect("lock");
        let gql_calls = vec.iter().filter(|c| c.ends_with("/graphql")).count();
        vec.push(format!("{} {}", req.method(), req.uri().path()));
        let (status, body) = match req.uri().path() {
            "/graphql" if gql_calls == 0 => (StatusCode::OK, r#"{"data":{}}"#),
            "/graphql" if gql_calls == 1 => (
                StatusCode::OK,
                r#"{"data":{"node":{"pullRequestReviewThread":{"id":"t"}}}}"#,
            ),
            "/graphql" => (
                StatusCode::OK,
                r#"{"data":{"resolveReviewThread":{"clientMutationId":null}}}"#,
            ),
            "/repos/o/r/pulls/comments/1" => (StatusCode::OK, r#"{"node_id":"c"}"#),
            _ => (StatusCode::NOT_FOUND, "{}"),
        };
        Response::builder()
            .status(status)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("response")
    });
    tokio::task::spawn_blocking(move || {
        vk_cmd(addr)
            .args(["resolve", "https://github.com/o/r/pull/83#discussion_r1"])
            .assert()
            .success();
    })
    .await
    .expect("spawn blocking");
    shutdown.shutdown().await;
    assert_eq!(
        calls.lock().expect("lock").as_slice(),
        [
            "POST /graphql",
            "GET /repos/o/r/pulls/comments/1",
            "POST /graphql",
            "POST /graphql",
        ],
    );
}

#[cfg(feature = "unstable-rest-resolve")]
async fn run_reply_flow(
    rest_status: StatusCode,
    expect_success: bool,
) -> (Vec<String>, Vec<u8>, Vec<u8>) {
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
                r#"{"data":{"node":{"pullRequestReviewThread":{"id":"t"}}}}"#
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
    let (stdout, stderr) = tokio::task::spawn_blocking(move || {
        let output = vk_cmd(addr)
            .args([
                "resolve",
                "https://github.com/o/r/pull/83#discussion_r1",
                "-m",
                "done",
            ])
            .output()
            .expect("run command");
        if expect_success {
            assert!(output.status.success());
        } else {
            assert!(!output.status.success());
        }
        (output.stdout, output.stderr)
    })
    .await
    .expect("spawn blocking");
    shutdown.shutdown().await;
    (calls.lock().expect("lock").clone(), stdout, stderr)
}

#[cfg(feature = "unstable-rest-resolve")]
#[tokio::test]
#[rstest::rstest]
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
    StatusCode::INTERNAL_SERVER_ERROR,
    false,
    &["POST /repos/o/r/pulls/83/comments/1/replies"],
)]
async fn resolve_flows_reply_rest(
    #[case] rest_status: StatusCode,
    #[case] should_succeed: bool,
    #[case] expected: &'static [&'static str],
) {
    let (calls, stdout, stderr) = run_reply_flow(rest_status, should_succeed).await;
    let stdout = String::from_utf8_lossy(&stdout);
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(stdout.trim().is_empty(), "unexpected stdout: {stdout}");
    if should_succeed {
        assert!(
            predicate::str::is_empty().eval(&stderr),
            "unexpected stderr: {stderr}"
        );
    } else {
        assert!(
            predicate::str::contains("replies")
                .and(predicate::str::contains("500"))
                .eval(&stderr),
            "stderr: {stderr}"
        );
    }
    assert_eq!(calls, expected);
}

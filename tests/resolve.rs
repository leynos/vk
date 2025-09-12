//! End-to-end tests for the `vk resolve` sub-command.

use assert_cmd::prelude::*;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use std::sync::{Arc, Mutex};

mod utils;
use utils::{start_mitm, vk_cmd};

#[rstest::rstest]
#[case(None)]
#[tokio::test]
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

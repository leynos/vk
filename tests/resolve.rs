//! End-to-end tests for the `vk resolve` sub-command.

use assert_cmd::prelude::*;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use std::sync::{Arc, Mutex};

mod utils;
use utils::{start_mitm, vk_cmd};

#[rstest::rstest]
#[case(Some("done"), &["POST /repos/o/r/pulls/comments/1/replies", "POST /graphql"])]
#[case(None, &["POST /graphql"])]
#[tokio::test]
async fn resolve_flows(#[case] msg: Option<&'static str>, #[case] expected: &[&str]) {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let clone = Arc::clone(&calls);
    *handler.lock().expect("lock handler") = Box::new(move |req| {
        clone.lock().expect("lock").push(format!("{} {}", req.method(), req.uri().path()));
        let body = if req.uri().path() == "/graphql" {
            r#"{"data":{"resolveReviewThread":{"clientMutationId":null}}}"#
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
    assert_eq!(calls.lock().expect("lock").as_slice(), expected);
}


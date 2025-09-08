//! End-to-end tests for the `vk resolve` sub-command.

use assert_cmd::prelude::*;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use std::sync::{Arc, Mutex};

mod utils;
use utils::{start_mitm, vk_cmd};

#[tokio::test]
async fn resolve_posts_message_and_marks_resolved() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let clone = Arc::clone(&calls);
    *handler.lock().expect("lock handler") = Box::new(move |req| {
        clone
            .lock()
            .expect("lock")
            .push(format!("{} {}", req.method(), req.uri().path()));
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from("{}"))
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
            "POST /repos/o/r/pulls/comments/1/replies",
            "PUT /repos/o/r/pulls/comments/1/resolve",
        ]
    );
}

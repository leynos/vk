//! E2E tests for handling outdated review threads.

use assert_cmd::prelude::*;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use predicates::{prelude::*, str::contains};
use tokio::task;

mod utils;
use utils::{start_mitm, vk_cmd};

#[tokio::test]
async fn pr_omits_outdated_threads_by_default() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let threads_body = include_str!("fixtures/review_threads_outdated.json").to_string();
    let reviews_body = include_str!("fixtures/reviews_empty.json").to_string();
    let mut responses = vec![threads_body, reviews_body].into_iter();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = responses.next().expect("response");
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("build response")
    });

    task::spawn_blocking(move || {
        let mut cmd = vk_cmd(addr);
        cmd.args(["pr", "https://github.com/o/r/pull/1"])
            .assert()
            .success()
            .stdout(contains("No unresolved comments."));
    })
    .await
    .expect("spawn blocking");

    shutdown.shutdown().await;
}

#[tokio::test]
async fn pr_shows_outdated_when_flag_set() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let threads_body = include_str!("fixtures/review_threads_outdated.json").to_string();
    let reviews_body = include_str!("fixtures/reviews_empty.json").to_string();
    let mut responses = vec![threads_body, reviews_body].into_iter();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = responses.next().expect("response");
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("build response")
    });

    task::spawn_blocking(move || {
        let mut cmd = vk_cmd(addr);
        cmd.args(["pr", "https://github.com/o/r/pull/1", "-o"])
            .assert()
            .success()
            .stdout(contains("obsolete"));
    })
    .await
    .expect("spawn blocking");

    shutdown.shutdown().await;
}

#[tokio::test]
async fn pr_show_outdated_respects_file_filter() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let threads_body =
        include_str!("fixtures/review_threads_outdated_multiple_files.json").to_string();
    let reviews_body = include_str!("fixtures/reviews_empty.json").to_string();
    let mut responses = vec![threads_body, reviews_body].into_iter();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = responses.next().expect("response");
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("build response")
    });

    task::spawn_blocking(move || {
        let mut cmd = vk_cmd(addr);
        cmd.args(["pr", "https://github.com/o/r/pull/1", "-o", "file.rs"])
            .assert()
            .success()
            .stdout(contains("obsolete").and(contains("current").not()));
    })
    .await
    .expect("spawn blocking");

    shutdown.shutdown().await;
}

//! E2E tests for handling outdated review threads.

use assert_cmd::prelude::*;
use predicates::{prelude::*, str::contains};
use tokio::task;

mod utils;
use utils::{set_sequential_responder, start_mitm, vk_cmd};

#[tokio::test]
async fn pr_omits_outdated_threads_by_default() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let threads_body = include_str!("fixtures/review_threads_outdated.json").to_string();
    let reviews_body = include_str!("fixtures/reviews_empty.json").to_string();
    set_sequential_responder(&handler, vec![threads_body, reviews_body]);

    task::spawn_blocking(move || {
        let mut cmd = vk_cmd(addr);
        cmd.args(["pr", "https://github.com/o/r/pull/1"])
            .assert()
            .success()
            .stdout(contains("No unresolved comments."))
            .stderr(predicates::str::is_empty());
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
    set_sequential_responder(&handler, vec![threads_body, reviews_body]);

    task::spawn_blocking(move || {
        let mut cmd = vk_cmd(addr);
        cmd.args(["pr", "https://github.com/o/r/pull/1", "-o"])
            .assert()
            .success()
            .stdout(contains("obsolete"))
            .stderr(predicates::str::is_empty());
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
    set_sequential_responder(&handler, vec![threads_body, reviews_body]);

    task::spawn_blocking(move || {
        let mut cmd = vk_cmd(addr);
        cmd.args(["pr", "https://github.com/o/r/pull/1", "-o", "file.rs"])
            .assert()
            .success()
            .stdout(contains("obsolete").and(contains("current").not()))
            .stderr(predicates::str::is_empty());
    })
    .await
    .expect("spawn blocking");

    shutdown.shutdown().await;
}

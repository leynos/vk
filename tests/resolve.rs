//! End-to-end tests for the `vk resolve` sub-command.

#![cfg(feature = "unstable-rest-resolve")]

use assert_cmd::prelude::*;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use predicates::prelude::*;
use serde_json::Value;
use std::borrow::ToOwned;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

mod utils;
use utils::{start_mitm, start_mitm_capture, vk_cmd};

/// Scripted GraphQL comment page.
struct Page {
    end_cursor: Option<&'static str>,
    comment_id: u32,
    thread_id: &'static str,
}

impl Page {
    fn next(end_cursor: &'static str, comment_id: u32, thread_id: &'static str) -> Self {
        Self {
            end_cursor: Some(end_cursor),
            comment_id,
            thread_id,
        }
    }

    fn last_with(comment_id: u32, thread_id: &'static str) -> Self {
        Self {
            end_cursor: None,
            comment_id,
            thread_id,
        }
    }

    fn body(&self) -> String {
        self.end_cursor.map_or_else(
            || {
                format!(
                    r#"{{"data":{{"repository":{{"pullRequest":{{"reviewComments":{{"pageInfo":{{"endCursor":null,"hasNextPage":false}},"nodes":[{{"databaseId":{},"pullRequestReviewThread":{{"id":"{}"}}}}]}}}}}}}}}}"#,
                    self.comment_id,
                    self.thread_id,
                )
            },
            |cursor| {
                format!(
                    r#"{{"data":{{"repository":{{"pullRequest":{{"reviewComments":{{"pageInfo":{{"endCursor":"{cursor}","hasNextPage":true}},"nodes":[{{"databaseId":{},"pullRequestReviewThread":{{"id":"{}"}}}}]}}}}}}}}}}"#,
                    self.comment_id,
                    self.thread_id,
                )
            },
        )
    }
}

/// Drive `vk resolve` and assert pagination.
async fn run_resolve_flow(pages: Vec<Page>, expected_posts: usize) {
    let (addr, handler, shutdown) = start_mitm_capture().await.expect("start server");
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let pages = Arc::new(Mutex::new(VecDeque::from(pages)));
    let expected_after = Arc::new(Mutex::new(None::<String>));
    let calls_clone = Arc::clone(&calls);
    let pages_clone = Arc::clone(&pages);
    let expected_after_clone = Arc::clone(&expected_after);
    *handler.lock().expect("lock handler") = Box::new(move |req| {
        let mut vec = calls_clone.lock().expect("lock");
        vec.push(format!("{} {}", req.method(), req.uri().path()));
        let body = if req.uri().path() == "/graphql" {
            let mut after = expected_after_clone.lock().expect("lock after");
            let body_bytes = req.body().as_ref();
            let v: Value = serde_json::from_slice(body_bytes).expect("JSON body for /graphql");
            let got_after = v
                .pointer("/variables/after")
                .and_then(|x| x.as_str())
                .map(ToOwned::to_owned);
            match after.as_deref() {
                Some(cursor) => assert_eq!(
                    got_after.as_deref(),
                    Some(cursor),
                    "query must include variables.after={cursor}; got: {v}"
                ),
                None => assert!(
                    got_after.is_none(),
                    "first page query must not include variables.after; got: {v}"
                ),
            }
            let mut pages = pages_clone.lock().expect("lock pages");
            if pages.is_empty() {
                r#"{"data":{"resolveReviewThread":{"clientMutationId":null}}}"#.to_owned()
            } else {
                let page = pages.pop_front().expect("non-empty script");
                *after = page.end_cursor.map(std::string::ToString::to_string);
                page.body()
            }
        } else {
            "{}".to_owned()
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
        vec!["POST /graphql"; expected_posts].as_slice()
    );
}

#[tokio::test]
#[rstest::rstest]
#[case::no_pagination(vec![Page::last_with(1, "t")], 2)]
#[case::two_pages(vec![Page::next("c1", 2, "other"), Page::last_with(1, "t")], 3)]
async fn resolve_flows(#[case] pages: Vec<Page>, #[case] expected_posts: usize) {
    run_resolve_flow(pages, expected_posts).await;
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

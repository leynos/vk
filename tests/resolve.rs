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
use serde_json::Value;
use std::borrow::ToOwned;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

mod resolve_async_mitm;
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
        // One review-thread page holding a single thread whose sole comment
        // carries the scripted database id (`fullDatabaseId` is a BigInt
        // scalar, transported as a string).
        let page_info = self.end_cursor.map_or_else(
            || r#"{"endCursor":null,"hasNextPage":false}"#.to_owned(),
            |cursor| format!(r#"{{"endCursor":"{cursor}","hasNextPage":true}}"#),
        );
        format!(
            r#"{{"data":{{"repository":{{"pullRequest":{{"reviewThreads":{{"pageInfo":{page_info},"nodes":[{{"id":"{}","comments":{{"nodes":[{{"fullDatabaseId":"{}"}}],"pageInfo":{{"endCursor":null,"hasNextPage":false}}}}}}]}}}}}}}}}}"#,
            self.thread_id, self.comment_id,
        )
    }
}

/// Validate one GraphQL request and return its scripted response body.
fn resolve_graphql_response(
    req: &Request<Bytes>,
    expected_after: &mut Option<String>,
    pages: &mut VecDeque<Page>,
    expected_thread_id: &str,
) -> String {
    let v: Value = serde_json::from_slice(req.body().as_ref()).expect("JSON body for /graphql");
    let got_after = v
        .pointer("/variables/after")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned);
    match expected_after.as_deref() {
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
    if pages.is_empty() {
        assert_eq!(
            v.pointer("/operationName"),
            Some(&Value::String("ResolveReviewThreadMutation".into()))
        );
        assert_eq!(
            v.pointer("/variables/id"),
            Some(&Value::String(expected_thread_id.into()))
        );
        r#"{"data":{"resolveReviewThread":{"clientMutationId":null}}}"#.to_owned()
    } else {
        assert_eq!(
            v.pointer("/operationName"),
            Some(&Value::String("ThreadForCommentQuery".into()))
        );
        assert_eq!(
            v.pointer("/variables/owner"),
            Some(&Value::String("o".into()))
        );
        assert_eq!(
            v.pointer("/variables/name"),
            Some(&Value::String("r".into()))
        );
        assert_eq!(
            v.pointer("/variables/number"),
            Some(&Value::Number(83.into()))
        );
        let page = pages.pop_front().expect("non-empty script");
        *expected_after = page.end_cursor.map(std::string::ToString::to_string);
        page.body()
    }
}

/// Drive `vk resolve` and assert pagination.
async fn run_resolve_flow(pages: Vec<Page>, expected_posts: usize) {
    let (addr, handler, shutdown) = start_mitm_capture().await.expect("start server");
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let pages = Arc::new(Mutex::new(VecDeque::from(pages)));
    let expected_thread_id = pages
        .lock()
        .expect("lock scripted pages")
        .back()
        .expect("at least one thread page")
        .thread_id;
    let expected_after = Arc::new(Mutex::new(None::<String>));
    let calls_clone = Arc::clone(&calls);
    let pages_clone = Arc::clone(&pages);
    let expected_after_clone = Arc::clone(&expected_after);
    *handler.lock().expect("lock handler") = Box::new(move |req| {
        let mut vec = calls_clone.lock().expect("lock");
        vec.push(format!("{} {}", req.method(), req.uri().path()));
        let body = if req.uri().path() == "/graphql" {
            let mut after = expected_after_clone.lock().expect("lock after");
            let mut pages = pages_clone.lock().expect("lock pages");
            resolve_graphql_response(req, &mut after, &mut pages, expected_thread_id)
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

//! Scripted GraphQL pagination scenarios for `vk resolve`.
//!
//! These drive the review-thread traversal end to end through the captured
//! MITM server: [`resolve_graphql_response`] checks each request against the
//! script and answers it, and [`run_resolve_flow`] runs the command and
//! asserts the exact request sequence. The reply-path scenarios live in the
//! parent module and build their own fixtures inline.

use assert_cmd::prelude::*;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode};
use predicates::prelude::*;
use serde_json::Value;
use std::borrow::ToOwned;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::utils::{start_mitm_capture, vk_cmd};

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

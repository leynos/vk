//! Tests for review thread fetching helpers.

use super::*;
use crate::VkError;
use crate::api::GraphQLClient;
use crate::ref_parser::RepoInfo;
use crate::test_utils::{TestClient, start_server};
#[cfg(debug_assertions)]
use futures::FutureExt;
use rstest::{fixture, rstest};
#[cfg(debug_assertions)]
use std::panic::AssertUnwindSafe;
use std::sync::atomic::Ordering;

#[fixture]
fn repo() -> RepoInfo {
    RepoInfo {
        owner: "o".into(),
        name: "r".into(),
    }
}

#[fixture]
async fn missing_nodes_client() -> TestClient {
    let body = serde_json::json!({
        "data": {
            "repository": {
                "pullRequest": {
                    "reviewThreads": {
                        "pageInfo": { "hasNextPage": false, "endCursor": null }
                    }
                }
            }
        }
    })
    .to_string();
    // Serve enough identical pages to cover retries/pagination during the test.
    start_server(vec![body.clone(); 6])
}

fn comment(body: &str) -> serde_json::Value {
    serde_json::json!({
        "body": body,
        "diffHunk": "",
        "originalPosition": null,
        "position": null,
        "path": "f",
        "url": "",
        "author": null
    })
}

#[fixture]
async fn pagination_client() -> TestClient {
    let first: Vec<_> = (0..100).map(|i| comment(&format!("c{i}"))).collect();
    let thread_body = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [{
                "id": "t",
                "isResolved": false,
                "comments": {"nodes": first, "pageInfo": {"hasNextPage": true, "endCursor": "c99"}}
            }],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}}
    })
    .to_string();
    let comment_body = serde_json::json!({
        "data": {"node": {"comments": {
            "nodes": [comment("c100")],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}
    })
    .to_string();
    start_server(vec![thread_body, comment_body])
}

#[allow(clippy::unused_async, reason = "rstest requires async fixtures")]
#[fixture]
async fn path_variant_client(
    #[default(serde_json::Value::Null)] path_value: serde_json::Value,
) -> TestClient {
    let body = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [{
                "id": "t",
                "isResolved": false,
                "comments": {"nodes": [{
                    "body": "c",
                    "diffHunk": "",
                    "originalPosition": null,
                    "position": null,
                    "path": path_value,
                    "url": "",
                    "author": null
                }], "pageInfo": {"hasNextPage": false, "endCursor": null}}
            }],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}}
    })
    .to_string();
    start_server(vec![body])
}

#[rstest]
#[tokio::test]
async fn run_query_missing_nodes_reports_path(
    repo: RepoInfo,
    #[future] missing_nodes_client: TestClient,
) {
    let TestClient { client, join, .. } = missing_nodes_client.await;
    let result = fetch_review_threads(&client, &repo, 1).await;
    let err = result.expect_err("expected error");
    let VkError::BadResponseSerde {
        status,
        message,
        snippet,
    } = err
    else {
        panic!("unexpected error: {err:?}");
    };
    assert_eq!(status, 200);
    assert!(
        message.contains("repository.pullRequest.reviewThreads"),
        "{message}"
    );
    assert!(!snippet.is_empty(), "JSON snippet should be captured");
    join.abort();
    let _ = join.await;
}

#[rstest]
#[case::empty("")]
#[case::whitespace(" ")]
#[tokio::test]
async fn comment_path_validation_error(
    repo: RepoInfo,
    #[case] path_value: &str,
    #[future]
    #[with(serde_json::Value::String(path_value.to_string()))]
    path_variant_client: TestClient,
) {
    let TestClient { client, join, .. } = path_variant_client.await;
    let err = fetch_review_threads(&client, &repo, 1)
        .await
        .expect_err("expected error");
    match err {
        VkError::EmptyCommentPath { thread_id, index } => {
            assert_eq!(thread_id.as_ref(), "t");
            assert_eq!(index, 0, "unexpected index for {path_value:?}");
        }
        other => panic!("unexpected error: {other}"),
    }
    join.abort();
    let _ = join.await;
}

#[rstest]
#[tokio::test]
async fn null_comment_path_is_error(
    repo: RepoInfo,
    #[future]
    #[with(serde_json::Value::Null)]
    path_variant_client: TestClient,
) {
    let TestClient { client, join, .. } = path_variant_client.await;
    let err = fetch_review_threads(&client, &repo, 1)
        .await
        .expect_err("expected error");
    assert!(matches!(
        err,
        VkError::BadResponseSerde {
            status: _,
            message: _,
            snippet: _
        }
    ));
    join.abort();
    let _ = join.await;
}

#[test]
fn filter_threads_by_files_retains_matches() {
    let threads = vec![
        ReviewThread {
            comments: CommentConnection {
                nodes: vec![ReviewComment {
                    path: "src/lib.rs".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        },
        ReviewThread {
            comments: CommentConnection {
                nodes: vec![ReviewComment {
                    path: "README.md".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        },
    ];
    let files = vec![String::from("README.md")];
    let filtered = filter_threads_by_files(threads, &files);
    assert_eq!(filtered.len(), 1);
    let path = filtered
        .first()
        .and_then(|t| t.comments.nodes.first())
        .map(|c| c.path.as_str());
    assert_eq!(path, Some("README.md"));
}

#[test]
fn retains_only_unresolved_threads() {
    let threads = vec![
        ReviewThread {
            is_resolved: true,
            ..Default::default()
        },
        ReviewThread {
            is_resolved: false,
            ..Default::default()
        },
    ];
    let filtered = filter_unresolved_threads(threads);
    assert_eq!(filtered.len(), 1);
    assert!(filtered.first().is_some_and(|t| !t.is_resolved));
}

#[rstest]
#[tokio::test]
async fn threads_with_many_comments_do_not_duplicate_first_page(
    repo: RepoInfo,
    #[future] pagination_client: TestClient,
) {
    let TestClient { client, join, .. } = pagination_client.await;
    let threads = fetch_review_threads(&client, &repo, 1)
        .await
        .expect("fetch threads");
    let thread = threads.first().expect("thread");
    assert_eq!(thread.comments.nodes.len(), 101);
    let bodies: Vec<_> = thread
        .comments
        .nodes
        .iter()
        .map(|c| c.body.clone())
        .collect();
    assert_eq!(
        bodies,
        (0..=100).map(|i| format!("c{i}")).collect::<Vec<_>>()
    );
    join.abort();
    let _ = join.await;
}

#[fixture]
async fn retry_client() -> TestClient {
    let page1 = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [{
                "id": "t1",
                "isResolved": false,
                "comments": {"nodes": [], "pageInfo": {"hasNextPage": false, "endCursor": null}}
            }],
            "pageInfo": {"hasNextPage": true, "endCursor": "c1"},
        }}}}
    })
    .to_string();
    let error = "{}".to_string();
    let page2 = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [{
                "id": "t2",
                "isResolved": false,
                "comments": {"nodes": [], "pageInfo": {"hasNextPage": false, "endCursor": null}}
            }],
            "pageInfo": {"hasNextPage": false, "endCursor": null},
        }}}}
    })
    .to_string();
    start_server(vec![page1, error, page2])
}

#[rstest]
#[tokio::test]
async fn retries_bad_page_and_preserves_order(repo: RepoInfo, #[future] retry_client: TestClient) {
    let TestClient { client, join, hits } = retry_client.await;
    let threads = fetch_review_threads(&client, &repo, 1)
        .await
        .expect("fetch threads");
    let ids: Vec<_> = threads.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["t1", "t2"]);
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    join.abort();
    let _ = join.await;
}

#[rstest]
#[tokio::test]
async fn rejects_out_of_range_number(repo: RepoInfo) {
    let client = GraphQLClient::new("token", None).expect("client");
    let number = i32::MAX as u64 + 1;
    if cfg!(debug_assertions) {
        let result = AssertUnwindSafe(fetch_review_threads(&client, &repo, number))
            .catch_unwind()
            .await;
        assert!(result.is_err());
        return;
    }
    let err = fetch_review_threads(&client, &repo, number)
        .await
        .expect_err("error");
    assert!(matches!(err, VkError::InvalidNumber));
}

#[rstest]
#[tokio::test]
async fn accepts_max_i32_number(repo: RepoInfo) {
    // Minimal valid response with no threads.
    let body = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [],
            "pageInfo": { "hasNextPage": false, "endCursor": null }
        }}}}
    })
    .to_string();
    let TestClient { client, join, hits } = start_server(vec![body]);
    let threads = fetch_review_threads(&client, &repo, i32::MAX as u64)
        .await
        .expect("should accept i32::MAX");
    assert!(threads.is_empty());
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "unexpected number of HTTP calls"
    );
    join.abort();
    let _ = join.await;
}

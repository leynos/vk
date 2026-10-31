//! Tests for review thread fetching helpers.

use super::*;
use crate::VkError;
use crate::ref_parser::RepoInfo;
use crate::test_utils::{TestClient, start_server};
use rstest::{fixture, rstest};

#[fixture]
pub(super) fn repo() -> RepoInfo {
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

pub(super) fn comment(body: &str) -> serde_json::Value {
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
pub(super) async fn pagination_client() -> TestClient {
    let first: Vec<_> = (0..100).map(|i| comment(&format!("c{i}"))).collect();
    let thread_body = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [{
                "id": "t",
                "isResolved": false,
                "isOutdated": false,
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

#[expect(
    clippy::unused_async,
    reason = "fixture remains async for symmetry with other variants"
)]
#[allow(
    unfulfilled_lint_expectations,
    reason = "clippy::unused_async may not trigger in all toolchains"
)]
#[fixture]
async fn path_variant_client(
    #[default(serde_json::Value::Null)] path_value: serde_json::Value,
) -> TestClient {
    // Intentionally async: other variants await I/O.
    let body = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [{
                "id": "t",
                "isResolved": false,
                "isOutdated": false,
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
async fn missing_nodes_are_treated_as_empty(
    repo: RepoInfo,
    #[future] missing_nodes_client: TestClient,
) {
    let TestClient { client, join, .. } = missing_nodes_client.await;
    let result =
        fetch_review_threads_with_options(&client, &repo, 1, FetchOptions::unresolved_current())
            .await;
    assert!(result.expect("missing nodes are empty").is_empty());
    join.abort();
    let _ = join.await;
}

#[test]
fn thread_for_comment_returns_slice_starting_at_id() {
    let threads = vec![ReviewThread {
        comments: CommentConnection {
            nodes: vec![
                ReviewComment {
                    url: "https://example.com#discussion_r1".into(),
                    ..Default::default()
                },
                ReviewComment {
                    body: "second".into(),
                    url: "https://example.com#discussion_r2".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        ..Default::default()
    }];
    let thread = thread_for_comment(threads, 2).expect("thread present");
    assert_eq!(thread.comments.nodes.len(), 1);
    let body = thread.comments.nodes.first().map(|c| c.body.as_str());
    assert_eq!(body, Some("second"));
}

#[test]
fn thread_for_comment_returns_none_for_missing_id() {
    let threads = vec![ReviewThread {
        comments: CommentConnection {
            nodes: vec![ReviewComment {
                url: "https://example.com#discussion_r1".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    }];
    assert!(thread_for_comment(threads, 2).is_none());
}

#[test]
fn thread_for_comment_picks_correct_thread_among_multiple() {
    let threads = vec![
        ReviewThread {
            comments: CommentConnection {
                nodes: vec![ReviewComment {
                    url: "https://example.com#discussion_r1".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        },
        ReviewThread {
            comments: CommentConnection {
                nodes: vec![ReviewComment {
                    url: "https://example.com#discussion_r2".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        },
    ];
    let thread = thread_for_comment(threads, 2).expect("thread present");
    assert_eq!(thread.comments.nodes.len(), 1);
    let url = thread.comments.nodes.first().map(|c| c.url.as_str());
    assert_eq!(url, Some("https://example.com#discussion_r2"));
}

#[rstest]
#[case(1, 2)]
#[case(2, 1)]
fn thread_for_comment_handles_position(#[case] id: u64, #[case] expected: usize) {
    let threads = vec![ReviewThread {
        comments: CommentConnection {
            nodes: vec![
                ReviewComment {
                    url: "https://example.com#discussion_r1".into(),
                    ..Default::default()
                },
                ReviewComment {
                    url: "https://example.com#discussion_r2".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
        ..Default::default()
    }];
    let thread = thread_for_comment(threads, id).expect("thread present");
    assert_eq!(thread.comments.nodes.len(), expected);
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
    let err =
        fetch_review_threads_with_options(&client, &repo, 1, FetchOptions::unresolved_current())
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
    let err =
        fetch_review_threads_with_options(&client, &repo, 1, FetchOptions::unresolved_current())
            .await
            .expect_err("expected error");
    let VkError::BadResponseSerde {
        status,
        message,
        snippet,
    } = err
    else {
        panic!("unexpected error: {err:?}");
    };
    assert_eq!(status, 200);
    assert!(message.contains("comments.nodes[0].path"), "{message}");
    assert!(snippet.contains("reviewThreads"), "{snippet}");
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

#[rstest]
#[case::mixed(
    vec![
        ReviewThread { is_outdated: true, ..Default::default() },
        ReviewThread { is_outdated: false, ..Default::default() },
    ],
    1,
)]
#[case::empty(vec![], 0)]
#[case::all_outdated(
    vec![
        ReviewThread { is_outdated: true, ..Default::default() },
        ReviewThread { is_outdated: true, ..Default::default() },
    ],
    0,
)]
#[case::none_outdated(
    vec![
        ReviewThread { is_outdated: false, ..Default::default() },
        ReviewThread { is_outdated: false, ..Default::default() },
    ],
    2,
)]
fn exclude_outdated_threads_filters(
    #[case] threads: Vec<ReviewThread>,
    #[case] expected_len: usize,
) {
    let filtered = super::exclude_outdated_threads(threads);
    assert_eq!(filtered.len(), expected_len);
    assert!(filtered.iter().all(|t| !t.is_outdated));
}

#[rstest]
#[case::outdated(
    |threads| super::filter_outdated_threads(threads),
    |thread: &mut ReviewThread, value| thread.is_outdated = value,
    |thread: &ReviewThread| thread.is_outdated,
    "outdated",
)]
#[case::unresolved(
    |threads| filter_unresolved_threads(threads),
    |thread: &mut ReviewThread, value| thread.is_resolved = value,
    |thread: &ReviewThread| thread.is_resolved,
    "resolved",
)]
fn filter_functions_exclude_matching_threads<F, S, G>(
    #[case] filter_fn: F,
    #[case] set_field: S,
    #[case] get_field: G,
    #[case] field_name: &str,
) where
    F: Fn(Vec<ReviewThread>) -> Vec<ReviewThread>,
    S: Fn(&mut ReviewThread, bool),
    G: Fn(&ReviewThread) -> bool,
{
    let mut threads = vec![ReviewThread::default(), ReviewThread::default()];
    let [first, second] = &mut threads[..] else {
        unreachable!()
    };
    set_field(first, true);
    set_field(second, false);

    let filtered = filter_fn(threads);
    assert_eq!(
        filtered.len(),
        1,
        "should retain exactly one thread for {field_name}"
    );
    assert!(
        !get_field(filtered.first().expect("thread present")),
        "remaining thread should have {field_name} = false"
    );
}

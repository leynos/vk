//! Tests for fetching review threads with options, retries and number bounds.

use super::tests::{comment, pagination_client, repo};
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
async fn retry_client() -> TestClient {
    let page1 = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [{
                "id": "t1",
                "isResolved": false,
                "isOutdated": false,
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
                "isOutdated": false,
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
    let TestClient {
        client, join, hits, ..
    } = retry_client.await;
    let threads =
        fetch_review_threads_with_options(&client, &repo, 1, FetchOptions::unresolved_current())
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
        let result = AssertUnwindSafe(fetch_review_threads_with_options(
            &client,
            &repo,
            number,
            FetchOptions::unresolved_current(),
        ))
        .catch_unwind()
        .await;
        assert!(result.is_err());
        return;
    }
    let err = fetch_review_threads_with_options(
        &client,
        &repo,
        number,
        FetchOptions::unresolved_current(),
    )
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
    let TestClient {
        client, join, hits, ..
    } = start_server(vec![body]);
    let threads = fetch_review_threads_with_options(
        &client,
        &repo,
        i32::MAX as u64,
        FetchOptions::unresolved_current(),
    )
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

#[rstest]
#[tokio::test]
async fn fetch_review_threads_with_options_can_include_resolved(repo: RepoInfo) {
    let body = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [
                {
                    "id": "t1",
                    "isResolved": true,
                    "isOutdated": false,
                    "comments": {"nodes": [comment("c1")], "pageInfo": {"hasNextPage": false, "endCursor": null}}
                },
                {
                    "id": "t2",
                    "isResolved": false,
                "isOutdated": false,
                    "comments": {"nodes": [comment("c2")], "pageInfo": {"hasNextPage": false, "endCursor": null}}
                }
            ],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}}
    }).to_string();
    let TestClient { client, join, .. } = start_server(vec![body.clone()]);
    let all = fetch_review_threads_with_options(
        &client,
        &repo,
        1,
        FetchOptions {
            include_resolved: true,
            include_outdated: true,
        },
    )
    .await
    .expect("fetch threads");
    assert_eq!(all.len(), 2);
    join.abort();
    let _ = join.await;

    let TestClient { client, join, .. } = start_server(vec![body]);
    let unresolved_only = fetch_review_threads_with_options(
        &client,
        &repo,
        1,
        FetchOptions {
            include_resolved: false,
            include_outdated: true,
        },
    )
    .await
    .expect("fetch threads");
    assert_eq!(unresolved_only.len(), 1);
    assert!(unresolved_only.first().is_some_and(|t| !t.is_resolved));
    join.abort();
    let _ = join.await;
}

#[rstest]
#[case::skip(false, vec!["t2"], 2)]
#[case::include(true, vec!["t1", "t2"], 3)]
#[tokio::test]
async fn fetch_review_threads_with_options_filters_outdated(
    repo: RepoInfo,
    #[case] include_outdated: bool,
    #[case] expected_ids: Vec<&'static str>,
    #[case] expected_hits: usize,
) {
    let threads_body = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [
                {
                    "id": "t1",
                    "isResolved": false,
                    "isOutdated": true,
                    "comments": {"nodes": [], "pageInfo": {"hasNextPage": true, "endCursor": "c1"}}
                },
                {
                    "id": "t2",
                    "isResolved": false,
                    "isOutdated": false,
                    "comments": {"nodes": [], "pageInfo": {"hasNextPage": true, "endCursor": "c2"}}
                }
            ],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}}
    })
    .to_string();
    let comment_body_c1 = serde_json::json!({
        "data": {"node": {"comments": {
            "nodes": [comment("c1")],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}
    })
    .to_string();
    let comment_body_c2 = serde_json::json!({
        "data": {"node": {"comments": {
            "nodes": [comment("c2")],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}
    })
    .to_string();
    let bodies = if include_outdated {
        vec![threads_body, comment_body_c1, comment_body_c2]
    } else {
        vec![threads_body, comment_body_c2]
    };
    let TestClient {
        client, join, hits, ..
    } = start_server(bodies);
    let threads = fetch_review_threads_with_options(
        &client,
        &repo,
        1,
        FetchOptions {
            include_resolved: false,
            include_outdated,
        },
    )
    .await
    .expect("fetch threads");
    assert_eq!(threads.len(), expected_ids.len());
    let ids: Vec<_> = threads.iter().map(|t| t.id.as_ref()).collect();
    for expected in expected_ids {
        assert!(ids.contains(&expected));
    }
    assert_eq!(hits.load(Ordering::SeqCst), expected_hits);
    join.abort();
    let _ = join.await;
}

#[rstest]
#[tokio::test]
async fn threads_with_many_comments_do_not_duplicate_first_page(
    repo: RepoInfo,
    #[future] pagination_client: TestClient,
) {
    let TestClient { client, join, .. } = pagination_client.await;
    let threads =
        fetch_review_threads_with_options(&client, &repo, 1, FetchOptions::unresolved_current())
            .await
            .expect("fetch threads");
    let thread = threads.first().expect("thread present");
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

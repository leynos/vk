//! Tests for review fetching and de-duplication.

use super::*;
use crate::ref_parser::RepoInfo;
use crate::test_utils::{TestClient, start_server};
use crate::{GraphQLClient, User, VkError};
// Explicit `chrono::DateTime` shadows the `scalars::DateTime` alias pulled
// in by `use super::*`, so the `DateTime<Utc>` case parameters below resolve
// to the generic chrono type.
use chrono::{DateTime, TimeZone, Utc};
use rstest::rstest;

#[rstest]
#[case(0)]
#[case(1)]
#[case(2)]
fn preserves_anonymous_reviews(#[case] count: usize) {
    let reviews = (0..count)
        .map(|i| {
            #[expect(clippy::cast_possible_wrap, reason = "indices are small")]
            let ts = i as i64 + 1;
            PullRequestReview {
                body: String::new(),
                submitted_at: Some(Utc.timestamp_opt(ts, 0).single().expect("timestamp")),
                state: "COMMENTED".into(),
                author: None,
            }
        })
        .collect();

    let latest = latest_reviews(reviews);
    assert_eq!(
        latest.len(),
        count,
        "anonymous reviews must be kept individually, got {latest:?}"
    );
}

#[rstest]
#[case(
    Some(Utc.timestamp_opt(10, 0).single().expect("ts")),
    Some(Utc.timestamp_opt(20, 0).single().expect("ts"))
)]
#[case(
    Some(Utc.timestamp_opt(10, 0).single().expect("ts")),
    Some(Utc.timestamp_opt(10, 0).single().expect("ts"))
)]
fn keeps_latest_per_author(
    #[case] first_ts: Option<DateTime<Utc>>,
    #[case] second_ts: Option<DateTime<Utc>>,
) {
    let a1 = PullRequestReview {
        body: "first".into(),
        submitted_at: first_ts,
        state: "COMMENTED".into(),
        author: Some(User {
            login: "alice".into(),
        }),
    };
    let a2 = PullRequestReview {
        body: "second".into(),
        submitted_at: second_ts,
        state: "COMMENTED".into(),
        author: Some(User {
            login: "alice".into(),
        }),
    };
    let b1 = PullRequestReview {
        body: String::new(),
        submitted_at: Some(Utc.timestamp_opt(30, 0).single().expect("ts")),
        state: "CHANGES_REQUESTED".into(),
        author: Some(User {
            login: "bob".into(),
        }),
    };
    let latest = latest_reviews(vec![a1, a2.clone(), b1.clone()]);
    let alice = latest
        .iter()
        .find(|r| r.author.as_ref().is_some_and(|u| u.login == "alice"))
        .expect("alice's review survives");
    // The body distinguishes which of alice's reviews won: the timestamps are
    // equal in one case, where only the input-order tie-break separates them.
    assert_eq!(
        alice.body, "second",
        "alice's surviving review must be the later input, got {latest:?}"
    );
    assert_eq!(
        alice.submitted_at, second_ts,
        "alice's surviving review must carry the later timestamp, got {latest:?}"
    );
    assert!(
        latest.iter().any(|r| {
            r.author.as_ref().is_some_and(|u| u.login == "bob") && r.submitted_at == b1.submitted_at
        }),
        "bob's review must survive, got {latest:?}"
    );
}

/// Build one page body from review nodes and the server's pagination state.
fn page_body(nodes: &serde_json::Value, has_next: bool, end_cursor: Option<&str>) -> String {
    serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviews": {
            "nodes": nodes,
            "pageInfo": { "hasNextPage": has_next, "endCursor": end_cursor }
        }}}}
    })
    .to_string()
}

/// Render one review node as the reviews query returns it.
fn review_node(body: &str, state: &str, submitted_at: &str, login: &str) -> serde_json::Value {
    serde_json::json!({
        "body": body,
        "state": state,
        "submittedAt": submitted_at,
        "author": { "login": login },
    })
}

/// The failure this guards against: reading only the first page of reviews.
///
/// A listing is silently truncated at the API's page size unless the caller
/// follows `pageInfo`. Taking "the newest review" from page 1 then reports a
/// superseded review as current — here, an old `CHANGES_REQUESTED` instead of
/// the `APPROVED` on page 2. This pins that `fetch_reviews` walks every page
/// and that the collation sees the later page.
#[tokio::test]
async fn fetches_every_page_before_collating_latest_reviews() {
    let first_page = page_body(
        &serde_json::json!([review_node(
            "Superseded",
            "CHANGES_REQUESTED",
            "2024-01-01T00:00:00Z",
            "coderabbitai[bot]",
        )]),
        true,
        Some("cursor-1"),
    );
    let second_page = page_body(
        &serde_json::json!([review_node(
            "Current",
            "APPROVED",
            "2024-01-02T00:00:00Z",
            "coderabbitai[bot]",
        )]),
        false,
        None,
    );

    let TestClient {
        client,
        join,
        requests,
        ..
    } = start_server(vec![first_page, second_page]);

    let reviews = fetch_reviews(
        &client,
        &RepoInfo {
            owner: "o".into(),
            name: "n".into(),
        },
        1,
    )
    .await
    .expect("both pages are fetched");

    let latest = latest_reviews(reviews);
    assert_eq!(latest.len(), 1, "one review per author");
    assert_eq!(
        latest.first().expect("one review").state,
        "APPROVED",
        "the newest review across all pages must win"
    );

    // The cursor must advance: two requests, the second carrying page 1's
    // cursor. The guard is scoped to this block so it is released before the
    // await below.
    let second_cursor = {
        let requests = requests.lock().expect("lock requests");
        assert_eq!(requests.len(), 2, "both pages must be requested");
        requests
            .get(1)
            .and_then(|request| request.pointer("/variables/cursor").cloned())
    };
    assert_eq!(
        second_cursor,
        Some(serde_json::json!("cursor-1")),
        "the second request must resume from the returned cursor"
    );

    join.abort();
    let _ = join.await;
}

#[tokio::test]
async fn rejects_out_of_range_number() {
    let client = GraphQLClient::new("token", None).expect("client");
    let repo = RepoInfo {
        owner: "o".into(),
        name: "n".into(),
    };
    let number = i32::MAX as u64 + 1;
    let err = fetch_reviews(&client, &repo, number)
        .await
        .expect_err("error");
    assert!(
        matches!(err, VkError::InvalidNumber),
        "a pull-request number above i32::MAX must be rejected"
    );
}

#[tokio::test]
async fn accepts_max_i32_number() {
    // Minimal valid response with no reviews.
    let body = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviews": {
            "nodes": [],
            "pageInfo": { "hasNextPage": false, "endCursor": null }
        }}}}
    })
    .to_string();
    let TestClient {
        client,
        join,
        requests,
        ..
    } = start_server(vec![body]);
    let reviews = fetch_reviews(
        &client,
        &RepoInfo {
            owner: "o".into(),
            name: "n".into(),
        },
        i32::MAX as u64,
    )
    .await
    .expect("should accept i32::MAX");
    assert!(
        reviews.is_empty(),
        "an empty reviews page must yield no reviews, got {reviews:?}"
    );
    {
        let requests = requests.lock().expect("lock requests");
        let request = requests.first().expect("one request");
        assert_eq!(
            request.get("operationName"),
            Some(&serde_json::json!("ReviewsQuery")),
            "the request must name the reviews operation"
        );
        assert_eq!(
            request.pointer("/variables/owner"),
            Some(&serde_json::json!("o")),
            "the owner must reach the request variables"
        );
        assert_eq!(
            request.pointer("/variables/name"),
            Some(&serde_json::json!("n")),
            "the repository name must reach the request variables"
        );
        assert_eq!(
            request.pointer("/variables/number"),
            Some(&serde_json::json!(i32::MAX)),
            "i32::MAX must be sent as the pull-request number"
        );
        assert_eq!(
            request.pointer("/variables/cursor"),
            Some(&serde_json::Value::Null),
            "the first request must start without a cursor"
        );
    }
    join.abort();
    let _ = join.await;
}

#[tokio::test]
async fn deserializes_null_submitted_at() {
    let body = include_str!("../../tests/fixtures/reviews_null_date.json");
    let TestClient { client, join, .. } = start_server(vec![body.to_string()]);
    let reviews = fetch_reviews(
        &client,
        &RepoInfo {
            owner: "o".into(),
            name: "n".into(),
        },
        1,
    )
    .await
    .expect("should accept null submittedAt");
    assert_eq!(
        reviews.len(),
        2,
        "both fixture reviews must be read, got {reviews:?}"
    );
    assert!(
        reviews.iter().any(|r| r.submitted_at.is_none()),
        "the pending review's null submittedAt must deserialise, got {reviews:?}"
    );
    assert!(
        reviews.iter().any(|r| r.submitted_at.is_some()),
        "the submitted review's timestamp must deserialise, got {reviews:?}"
    );
    join.abort();
    let _ = join.await;
}

#[test]
fn latest_reviews_prefers_timestamp_over_none() {
    let ts = Utc.timestamp_opt(100, 0).single().expect("ts");
    let with_ts = PullRequestReview {
        body: "with timestamp".into(),
        submitted_at: Some(ts),
        state: "APPROVED".into(),
        author: Some(User {
            login: "alice".into(),
        }),
    };
    let without_ts = PullRequestReview {
        body: "without timestamp".into(),
        submitted_at: None,
        state: "PENDING".into(),
        author: Some(User {
            login: "alice".into(),
        }),
    };

    // Regardless of order, the one with the timestamp should win.
    let latest = latest_reviews(vec![with_ts.clone(), without_ts.clone()]);
    assert_eq!(
        latest.len(),
        1,
        "one review per author must survive, got {latest:?}"
    );
    assert_eq!(
        latest.first().expect("one review").submitted_at,
        Some(ts),
        "the timestamped review must win when it comes first, got {latest:?}"
    );

    let latest = latest_reviews(vec![without_ts, with_ts]);
    assert_eq!(
        latest.len(),
        1,
        "one review per author must survive, got {latest:?}"
    );
    assert_eq!(
        latest.first().expect("one review").submitted_at,
        Some(ts),
        "the timestamped review must win when it comes last, got {latest:?}"
    );
}

#[test]
fn latest_reviews_both_none_takes_later_in_input() {
    let first = PullRequestReview {
        body: "first".into(),
        submitted_at: None,
        state: "PENDING".into(),
        author: Some(User {
            login: "alice".into(),
        }),
    };
    let second = PullRequestReview {
        body: "second".into(),
        submitted_at: None,
        state: "PENDING".into(),
        author: Some(User {
            login: "alice".into(),
        }),
    };
    let latest = latest_reviews(vec![first, second]);
    assert_eq!(
        latest.len(),
        1,
        "one review per author must survive, got {latest:?}"
    );
    assert_eq!(
        latest.first().expect("one review").body,
        "second",
        "the later input must win when neither review has a timestamp, got {latest:?}"
    );
}

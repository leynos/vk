//! Functions for retrieving pull-request reviews through the GitHub API.
//!
//! The module defines GraphQL query structures and pagination helpers so callers
//! can fetch pull-request reviews and collate the latest review from each author.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Map, json};

use crate::{GraphQLClient, PageInfo, User, VkError, ref_parser::RepoInfo};
use std::collections::{HashMap, hash_map::Entry};

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestReview {
    pub body: String,
    /// Timestamp when the review was formally submitted.
    ///
    /// This may be `None` when the timestamp is missing or unknown.
    pub submitted_at: Option<DateTime<Utc>>,
    pub state: String,
    pub author: Option<User>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewData {
    repository: RepositoryReviews,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryReviews {
    #[serde(rename = "pullRequest")]
    pull_request: PullRequestReviews,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestReviews {
    reviews: ReviewConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewConnection {
    nodes: Vec<PullRequestReview>,
    page_info: PageInfo,
}

const REVIEWS_QUERY: &str = r"
    query($owner: String!, $name: String!, $number: Int!, $cursor: String) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          reviews(first: 100, after: $cursor) {
            nodes {
              body
              state
              submittedAt
              author { login }
            }
            pageInfo { hasNextPage endCursor }
          }
        }
      }
    }
";

/// Retrieve all reviews for a pull request by paging through the GitHub
/// GraphQL API.
///
/// Note:
/// - GitHub GraphQL `Int` is a 32-bit signed integer (range −2^31..=2^31−1).
///   This function accepts a non-negative `number`; values above `i32::MAX`
///   are rejected with [`VkError::InvalidNumber`].
/// - The token must have sufficient scopes (for example, `repo` for private
///   repositories) or the API may return partial data that fails to
///   deserialise.
///
/// ```no_run
/// use vk::{GraphQLClient, ref_parser::RepoInfo};
///
/// # async fn run() -> Result<(), vk::VkError> {
/// let client = GraphQLClient::new("token", None).expect("client");
/// let repo = RepoInfo { owner: "octocat".into(), name: "hello-world".into() };
/// let reviews = vk::reviews::fetch_reviews(&client, &repo, 1).await?;
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns [`VkError::InvalidNumber`] if `number` exceeds `i32::MAX`, or a
/// [`VkError`] if the request fails or the response cannot be deserialised.
pub async fn fetch_reviews(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Vec<PullRequestReview>, VkError> {
    debug_assert!(
        i32::try_from(number).is_ok(),
        "pull-request number {number} exceeds GraphQL Int (i32) range",
    );
    let number_i32 = i32::try_from(number).map_err(|_| VkError::InvalidNumber)?;

    let mut vars = Map::new();
    vars.insert("owner".into(), json!(repo.owner.clone()));
    vars.insert("name".into(), json!(repo.name.clone()));
    vars.insert("number".into(), json!(number_i32));
    client
        .paginate_all(REVIEWS_QUERY, vars, None, |data: ReviewData| {
            let conn = data.repository.pull_request.reviews;
            Ok((conn.nodes, conn.page_info))
        })
        .await
}

/// Determine whether `new` should replace `existing` when collating reviews.
///
/// Prefer reviews with a timestamp over those without. When both have
/// timestamps, keep the later one. Tie-break on equal timestamps (or both
/// `None`) by favouring the later item in input order.
#[expect(
    clippy::match_same_arms,
    reason = "arms kept separate for readability of tie-breaking rules"
)]
fn is_dominated(new: &PullRequestReview, existing: &PullRequestReview) -> bool {
    match (new.submitted_at, existing.submitted_at) {
        (Some(new_ts), Some(old_ts)) => new_ts >= old_ts,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => true,
    }
}

/// Select the most recent review from each author.
///
/// Reviews without an author are returned individually rather than being
/// grouped together.
///
/// Ordering:
/// - The order of reviews with authors is not guaranteed.
/// - Anonymous reviews are appended after the keyed results.
///
/// Tie-break:
/// - When two reviews from the same author share the same `submitted_at`,
///   the later item in the input sequence wins.
///
/// If you require a deterministic order, sort the returned vector by
/// `submitted_at` at the call site.
///
/// ```
/// use chrono::Utc;
/// use vk::reviews::{latest_reviews, PullRequestReview};
///
/// let reviews = vec![
///     PullRequestReview {
///         body: String::new(),
///         submitted_at: Some(Utc::now()),
///         state: "COMMENTED".into(),
///         author: None,
///     },
///     PullRequestReview {
///         body: String::new(),
///         submitted_at: Some(Utc::now()),
///         state: "COMMENTED".into(),
///         author: None,
///     },
/// ];
/// let latest = latest_reviews(reviews);
/// assert_eq!(latest.len(), 2);
/// ```
pub fn latest_reviews(reviews: Vec<PullRequestReview>) -> Vec<PullRequestReview> {
    let mut latest: HashMap<String, PullRequestReview> = HashMap::new();
    let mut anonymous = Vec::new();
    for r in reviews {
        // GitHub canonicalises login casing, so case collisions are not
        // expected when keying by `login`.
        match r.author.as_ref().map(|u| u.login.clone()) {
            Some(login) => match latest.entry(login) {
                Entry::Vacant(e) => {
                    e.insert(r);
                }
                Entry::Occupied(mut e) => {
                    if is_dominated(&r, e.get()) {
                        e.insert(r);
                    }
                }
            },
            None => anonymous.push(r),
        }
    }
    let keyed = latest.into_values();
    let mut out = Vec::with_capacity(keyed.len() + anonymous.len());
    out.extend(keyed);
    out.extend(anonymous);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ref_parser::RepoInfo;
    use crate::test_utils::{TestClient, start_server};
    use crate::{GraphQLClient, User, VkError};
    use chrono::{TimeZone, Utc};
    #[cfg(debug_assertions)]
    use futures::FutureExt;
    use rstest::rstest;
    #[cfg(debug_assertions)]
    use std::panic::AssertUnwindSafe;

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
        assert_eq!(latest.len(), count);
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
        assert!(latest.iter().any(|r| {
            r.author.as_ref().expect("author").login == "alice" && r.submitted_at == a2.submitted_at
        }));
        assert!(latest.iter().any(|r| {
            r.author.as_ref().expect("author").login == "bob" && r.submitted_at == b1.submitted_at
        }));
    }

    #[tokio::test]
    async fn rejects_out_of_range_number() {
        let client = GraphQLClient::new("token", None).expect("client");
        let repo = RepoInfo {
            owner: "o".into(),
            name: "n".into(),
        };
        let number = i32::MAX as u64 + 1;
        if cfg!(debug_assertions) {
            let result = AssertUnwindSafe(fetch_reviews(&client, &repo, number))
                .catch_unwind()
                .await;
            assert!(result.is_err());
            return;
        }
        let err = fetch_reviews(&client, &repo, number)
            .await
            .expect_err("error");
        assert!(matches!(err, VkError::InvalidNumber));
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
        let TestClient { client, join, .. } = start_server(vec![body]);
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
        assert!(reviews.is_empty());
        join.abort();
        let _ = join.await;
    }

    #[tokio::test]
    async fn deserializes_null_submitted_at() {
        let body = include_str!("../tests/fixtures/reviews_null_date.json");
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
        assert_eq!(reviews.len(), 2);
        assert!(reviews.iter().any(|r| r.submitted_at.is_none()));
        assert!(reviews.iter().any(|r| r.submitted_at.is_some()));
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
        assert_eq!(latest.len(), 1);
        assert_eq!(latest.first().expect("one review").submitted_at, Some(ts));

        let latest = latest_reviews(vec![without_ts, with_ts]);
        assert_eq!(latest.len(), 1);
        assert_eq!(latest.first().expect("one review").submitted_at, Some(ts));
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
        assert_eq!(latest.len(), 1);
        assert_eq!(latest.first().expect("one review").body, "second");
    }
}

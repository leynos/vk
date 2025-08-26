//! Functions for retrieving pull-request reviews through the GitHub API.
//!
//! The module defines GraphQL query structures and pagination helpers so callers
//! can fetch review threads and collate the latest review from each author.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Map, json};

use crate::{GraphQLClient, PageInfo, User, VkError, ref_parser::RepoInfo};
use std::collections::{HashMap, hash_map::Entry};
use std::convert::TryFrom;

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestReview {
    pub body: String,
    pub submitted_at: DateTime<Utc>,
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
/// - GitHub GraphQL `Int` is a 32-bit signed integer; pass a pull-request
///   number within `i32::MAX`.
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

/// Select the most recent review from each author.
///
/// Reviews without an author are returned individually rather than being
/// grouped together.
///
/// Ordering:
/// - The order of reviews with authors is not guaranteed.
/// - Anonymous reviews are appended after the keyed results.
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
///         submitted_at: Utc::now(),
///         state: "COMMENTED".into(),
///         author: None,
///     },
///     PullRequestReview {
///         body: String::new(),
///         submitted_at: Utc::now(),
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
                    // Tie-break on equal timestamps by favouring the later
                    // item in input order.
                    if r.submitted_at >= e.get().submitted_at {
                        e.insert(r);
                    }
                }
            },
            None => anonymous.push(r),
        }
    }
    latest.into_values().chain(anonymous).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ref_parser::RepoInfo;
    use crate::{GraphQLClient, User, VkError};
    use chrono::{TimeZone, Utc};
    use rstest::rstest;
    use std::convert::TryFrom;

    #[rstest]
    #[case(1)]
    #[case(2)]
    fn preserves_anonymous_reviews(#[case] count: usize) {
        let reviews = (0..count)
            .map(|i| {
                let ts = i64::try_from(i).expect("index") + 1;
                PullRequestReview {
                    body: String::new(),
                    submitted_at: Utc.timestamp_opt(ts, 0).single().expect("timestamp"),
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
        Utc.timestamp_opt(10, 0).single().expect("ts"),
        Utc.timestamp_opt(20, 0).single().expect("ts")
    )]
    #[case(
        Utc.timestamp_opt(10, 0).single().expect("ts"),
        Utc.timestamp_opt(10, 0).single().expect("ts")
    )]
    fn keeps_latest_per_author(#[case] first_ts: DateTime<Utc>, #[case] second_ts: DateTime<Utc>) {
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
            submitted_at: Utc.timestamp_opt(30, 0).single().expect("ts"),
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
        let err = fetch_reviews(&client, &repo, number)
            .await
            .expect_err("error");
        assert!(matches!(err, VkError::InvalidNumber));
    }
}

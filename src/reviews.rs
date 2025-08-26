//! Functions for retrieving pull-request reviews through the GitHub API.
//!
//! The module defines GraphQL query structures and pagination helpers so callers
//! can fetch review threads and collate the latest review from each author.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Map, json};

use crate::{GraphQLClient, PageInfo, User, VkError, ref_parser::RepoInfo};
use std::collections::{HashMap, hash_map::Entry};

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
/// Returns a [`VkError`] if the request fails or the response cannot be
/// deserialised.
pub async fn fetch_reviews(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Vec<PullRequestReview>, VkError> {
    let mut vars = Map::new();
    vars.insert("owner".into(), json!(repo.owner.clone()));
    vars.insert("name".into(), json!(repo.name.clone()));
    vars.insert("number".into(), json!(number));
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
    use crate::User;
    use chrono::{TimeZone, Utc};

    #[test]
    fn preserves_anonymous_reviews() {
        let reviews = vec![
            PullRequestReview {
                body: String::new(),
                submitted_at: Utc.timestamp_opt(1, 0).single().expect("timestamp"),
                state: "COMMENTED".into(),
                author: None,
            },
            PullRequestReview {
                body: String::new(),
                submitted_at: Utc.timestamp_opt(2, 0).single().expect("timestamp"),
                state: "COMMENTED".into(),
                author: None,
            },
        ];

        let latest = latest_reviews(reviews);
        assert_eq!(latest.len(), 2);
    }

    #[test]
    fn keeps_latest_per_author() {
        let a1 = PullRequestReview {
            body: String::new(),
            submitted_at: Utc.timestamp_opt(10, 0).single().expect("ts"),
            state: "COMMENTED".into(),
            author: Some(User {
                login: "alice".into(),
            }),
        };
        let a2 = PullRequestReview {
            body: String::new(),
            submitted_at: Utc.timestamp_opt(20, 0).single().expect("ts"),
            state: "APPROVED".into(),
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
        let latest = latest_reviews(vec![a1.clone(), a2.clone(), b1.clone()]);
        assert!(latest.iter().any(|r| {
            r.author.as_ref().expect("author").login == "alice" && r.submitted_at == a2.submitted_at
        }));
        assert!(latest.iter().any(|r| {
            r.author.as_ref().expect("author").login == "bob" && r.submitted_at == b1.submitted_at
        }));
    }

    #[test]
    fn ties_favour_last_in_input_when_timestamps_equal() {
        let a1 = PullRequestReview {
            body: "first".into(),
            submitted_at: Utc.timestamp_opt(10, 0).single().expect("ts"),
            state: "COMMENTED".into(),
            author: Some(User {
                login: "alice".into(),
            }),
        };
        let a2 = PullRequestReview {
            body: "second".into(),
            submitted_at: Utc.timestamp_opt(10, 0).single().expect("ts"),
            state: "COMMENTED".into(),
            author: Some(User {
                login: "alice".into(),
            }),
        };
        let latest = latest_reviews(vec![a1, a2.clone()]);
        assert_eq!(latest.len(), 1);
        assert_eq!(latest.first().expect("review").body, "second");
    }
}

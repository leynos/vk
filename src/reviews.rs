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
                    if r.submitted_at > e.get().submitted_at {
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
}

//! Functions for retrieving pull-request reviews through the GitHub API.
//!
//! The module defines GraphQL query structures and pagination helpers so callers
//! can fetch review threads and collate the latest review from each author.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::api::{self, GraphQLClient};
use crate::{PageInfo, RepoInfo, User, VkError};
use std::collections::HashMap;

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

pub async fn fetch_review_page(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
    cursor: Option<String>,
) -> Result<(Vec<PullRequestReview>, PageInfo), VkError> {
    let data: ReviewData = api::run_query(
        client,
        REVIEWS_QUERY,
        json!({
            "owner": repo.owner.as_str(),
            "name": repo.name.as_str(),
            "number": number,
            "cursor": cursor,
        }),
    )
    .await?;
    let conn = data.repository.pull_request.reviews;
    Ok((conn.nodes, conn.page_info))
}

pub async fn fetch_reviews(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Vec<PullRequestReview>, VkError> {
    api::paginate(|c| fetch_review_page(client, repo, number, c)).await
}

pub fn latest_reviews(reviews: Vec<PullRequestReview>) -> Vec<PullRequestReview> {
    let mut latest: HashMap<String, PullRequestReview> = HashMap::new();
    for r in reviews {
        let key = r
            .author
            .as_ref()
            .map(|u| u.login.clone())
            .unwrap_or_default();
        match latest.entry(key) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(r);
            }
            std::collections::hash_map::Entry::Occupied(mut e) => {
                if r.submitted_at > e.get().submitted_at {
                    e.insert(r);
                }
            }
        }
    }
    latest.into_values().collect()
}

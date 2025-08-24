//! Functions for retrieving pull-request reviews through the GitHub API.
//!
//! The module defines GraphQL query structures and pagination helpers so callers
//! can fetch review threads and collate the latest review from each author.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Map, json};

use crate::{GraphQLClient, PageInfo, User, VkError, ref_parser::RepoInfo};
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

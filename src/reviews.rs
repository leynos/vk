//! Helpers for fetching and displaying pull request reviews.

use crate::html::collapse_details;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use termimad::MadSkin;

use crate::{GraphQLClient, PageInfo, RepoInfo, User, VkError};
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
    pull_request: ReviewConnection,
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
    let data: ReviewData = client
        .run_query(
            REVIEWS_QUERY,
            json!({
                "owner": repo.owner.as_str(),
                "name": repo.name.as_str(),
                "number": number,
                "cursor": cursor,
            }),
        )
        .await?;
    let conn = data.repository.pull_request;
    Ok((conn.nodes, conn.page_info))
}

pub async fn fetch_reviews(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Vec<PullRequestReview>, VkError> {
    crate::paginate(|c| fetch_review_page(client, repo, number, c)).await
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

pub fn write_review<W: std::io::Write>(
    mut out: W,
    skin: &MadSkin,
    review: &PullRequestReview,
) -> anyhow::Result<()> {
    let author = review
        .author
        .as_ref()
        .map_or("(unknown)", |u| u.login.as_str());
    writeln!(out, "\u{1f4dd}  \x1b[1m{author}\x1b[0m {}:", review.state)?;
    let body = collapse_details(&review.body);
    if let Err(e) = skin.write_text_on(&mut out, &body) {
        eprintln!("error writing review body: {e}");
    }
    writeln!(out)?;
    Ok(())
}

pub fn print_reviews(skin: &MadSkin, reviews: &[PullRequestReview]) {
    for r in reviews {
        if let Err(e) = write_review(std::io::stdout().lock(), skin, r) {
            eprintln!("error printing review: {e}");
        }
    }
}

//! Helpers for fetching and displaying pull request reviews.

use crate::api::{GraphQLClient, VkError};
use crate::html::collapse_details;
use crate::models::{PageInfo, User};
use crate::references::RepoInfo;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use termimad::MadSkin;

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

/// Fetch a single page of reviews from GitHub.
///
/// # Errors
///
/// Returns `VkError` if the query fails.
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

/// Fetch all reviews for the specified pull request.
///
/// # Errors
///
/// Returns `VkError` if any page fails to load.
pub async fn fetch_reviews(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Vec<PullRequestReview>, VkError> {
    crate::api::paginate(|c| fetch_review_page(client, repo, number, c)).await
}

/// Keep only the most recent review from each author.
#[must_use]
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

/// Write a single review body to the provided writer.
///
/// # Errors
///
/// Returns an error if writing to `out` fails.
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

/// Print multiple reviews to stdout.
pub fn print_reviews(skin: &MadSkin, reviews: &[PullRequestReview]) {
    for r in reviews {
        if let Err(e) = write_review(std::io::stdout().lock(), skin, r) {
            eprintln!("error printing review: {e}");
        }
    }
}

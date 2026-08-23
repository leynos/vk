//! Helpers for fetching issues from the GitHub API.
//!
//! Currently only retrieval of a single issue by number is supported.

use crate::graphql_queries::ISSUE_QUERY;
use crate::ref_parser::RepoInfo;
use crate::{GraphQLClient, VkError};
use serde::Deserialize;
use serde_json::{Map, json};

/// GraphQL data returned for an issue lookup.
#[derive(Deserialize)]
struct IssueData {
    /// Repository containing the requested issue.
    repository: IssueRepository,
}

/// Repository portion of an issue lookup response.
#[derive(Deserialize)]
struct IssueRepository {
    /// Requested issue.
    issue: Issue,
}

/// Minimal issue representation returned by the GitHub API.
#[derive(Deserialize)]
///
/// The title and body are retained for display by the command layer.
pub struct Issue {
    /// Issue title.
    pub title: String,
    /// Issue body in Markdown.
    pub body: String,
}

/// Fetch a single issue by repository and number.
///
/// # Errors
///
/// Returns an error if the API request fails or the response is malformed.
pub async fn fetch_issue(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Issue, VkError> {
    let mut vars = Map::new();
    vars.insert("owner".into(), json!(repo.owner.clone()));
    vars.insert("name".into(), json!(repo.name.clone()));
    vars.insert("number".into(), json!(number));
    let data: IssueData = client.fetch_page(ISSUE_QUERY, None, vars).await?;
    Ok(data.repository.issue)
}

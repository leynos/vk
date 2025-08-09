//! Helpers for fetching issues from the GitHub API.
//!
//! Currently only retrieval of a single issue by number is supported.

use serde::Deserialize;
use serde_json::json;

use crate::graphql_queries::ISSUE_QUERY;
use crate::ref_parser::RepoInfo;
use crate::{GraphQLClient, VkError};

#[derive(Deserialize)]
struct IssueData {
    repository: IssueRepository,
}

#[derive(Deserialize)]
struct IssueRepository {
    issue: Issue,
}

/// Minimal issue representation returned by the GitHub API.
#[derive(Deserialize)]
pub struct Issue {
    pub title: String,
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
    let data: IssueData = client
        .run_query(
            ISSUE_QUERY,
            json!({
                "owner": repo.owner.as_str(),
                "name": repo.name.as_str(),
                "number": number
            }),
        )
        .await?;
    Ok(data.repository.issue)
}

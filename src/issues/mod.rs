//! Helpers for fetching issues from the GitHub API.
//!
//! Currently only retrieval of a single issue by number is supported.
//!
//! The GraphQL operation itself lives in the private [`wire`] submodule, which
//! owns the `graphql_client` codegen item, the generated-variable construction,
//! the response mapping, and the [`GraphQLClient`] call. This module owns the
//! public [`Issue`] domain type and the [`fetch_issue`] entry point, and speaks
//! only that type across the boundary.

use crate::ref_parser::RepoInfo;
use crate::{GraphQLClient, VkError};

mod wire;

/// Minimal issue representation returned by the GitHub API.
///
/// `Deserialize` is retained for consumers that decode fixtures directly; the
/// GraphQL path populates this type through the private wire adapter.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
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
/// Returns an error if the issue number is out of range, the API request
/// fails, or the response is malformed or reports no such issue.
pub async fn fetch_issue(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Issue, VkError> {
    wire::fetch_issue(client, repo, number).await
}

#[cfg(test)]
mod tests;

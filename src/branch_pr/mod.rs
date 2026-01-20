//! Utilities for resolving pull requests from the current Git branch.
//!
//! This module provides functions to look up pull requests associated with a
//! branch via the GitHub GraphQL API. Supports disambiguation when multiple
//! forks have PRs with the same branch name by filtering on the head repository
//! owner.

use serde::Deserialize;
use serde_json::{Map, json};

use crate::graphql_queries::PR_FOR_BRANCH_QUERY;
use crate::ref_parser::RepoInfo;
use crate::{GraphQLClient, VkError};

#[derive(Debug, Deserialize)]
pub(crate) struct PrForBranchData {
    repository: PrForBranchRepository,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrForBranchRepository {
    pull_requests: PrConnection,
}

#[derive(Debug, Deserialize)]
struct PrConnection {
    nodes: Vec<PrNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrNode {
    pub(crate) number: u64,
    pub(crate) head_repository: Option<HeadRepository>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HeadRepository {
    pub(crate) owner: Owner,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Owner {
    pub(crate) login: String,
}

/// Look up the pull request number for a branch via the GitHub API.
///
/// Queries for open or merged PRs with the given branch as the head ref. When
/// `head_owner` is provided, filters results to match the head repository owner,
/// disambiguating between multiple forks with the same branch name.
///
/// # Arguments
///
/// * `client` - The GraphQL client for API requests
/// * `repo` - The target repository (where the PR is opened against)
/// * `branch` - The head branch name to search for
/// * `head_owner` - Optional owner of the head repository (the fork owner). When
///   `None`, returns the first matching PR without filtering.
///
/// # Errors
///
/// Returns [`VkError::NoPrForBranch`] if no PR exists for the branch (or no PR
/// matches the specified head owner), or propagates API errors from the
/// underlying request.
///
/// # Example
///
/// ```ignore
/// use crate::{GraphQLClient, VkError};
/// use crate::ref_parser::RepoInfo;
/// use crate::branch_pr::fetch_pr_for_branch;
///
/// async fn demo(client: &GraphQLClient) -> Result<(), VkError> {
///     let repo = RepoInfo {
///         owner: "upstream".into(),
///         name: "project".into(),
///     };
///     let pr_number = fetch_pr_for_branch(
///         client,
///         &repo,
///         "feature-branch",
///         Some("fork-owner"),
///     ).await?;
///     println!("Found PR #{pr_number}");
///     Ok(())
/// }
/// ```
pub async fn fetch_pr_for_branch(
    client: &GraphQLClient,
    repo: &RepoInfo,
    branch: &str,
    head_owner: Option<&str>,
) -> Result<u64, VkError> {
    let mut vars = Map::new();
    vars.insert("owner".into(), json!(repo.owner.clone()));
    vars.insert("name".into(), json!(repo.name.clone()));
    vars.insert("headRef".into(), json!(branch));

    let data: PrForBranchData = client.run_query(PR_FOR_BRANCH_QUERY, vars).await?;

    let prs = &data.repository.pull_requests.nodes;

    // If head_owner is specified, filter PRs by head repository owner.
    // When no head owner is provided, fall back to the first PR (backward compatible).
    let matching_pr = head_owner.map_or_else(
        || prs.first(),
        |owner| {
            prs.iter().find(|pr| {
                pr.head_repository
                    .as_ref()
                    .is_some_and(|hr| hr.owner.login.eq_ignore_ascii_case(owner))
            })
        },
    );

    matching_pr
        .map(|pr| pr.number)
        .ok_or_else(|| VkError::NoPrForBranch {
            branch: branch.into(),
        })
}

#[cfg(test)]
mod tests;

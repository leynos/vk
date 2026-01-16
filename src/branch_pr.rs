//! Utilities for resolving pull requests from the current Git branch.
//!
//! This module provides functions to look up pull requests associated with a
//! branch via the GitHub GraphQL API.

use serde::Deserialize;
use serde_json::{Map, json};

use crate::graphql_queries::PR_FOR_BRANCH_QUERY;
use crate::ref_parser::RepoInfo;
use crate::{GraphQLClient, VkError};

#[derive(Debug, Deserialize)]
struct PrForBranchData {
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
struct PrNode {
    number: u64,
}

/// Look up the pull request number for a branch via the GitHub API.
///
/// Queries for open or merged PRs with the given branch as the head ref.
/// Returns the PR number if a matching PR is found.
///
/// # Errors
///
/// Returns [`VkError::NoPrForBranch`] if no PR exists for the branch, or
/// propagates API errors from the underlying request.
pub async fn fetch_pr_for_branch(
    client: &GraphQLClient,
    repo: &RepoInfo,
    branch: &str,
) -> Result<u64, VkError> {
    let mut vars = Map::new();
    vars.insert("owner".into(), json!(repo.owner.clone()));
    vars.insert("name".into(), json!(repo.name.clone()));
    vars.insert("headRef".into(), json!(branch));

    let data: PrForBranchData = client.run_query(PR_FOR_BRANCH_QUERY, vars).await?;

    data.repository
        .pull_requests
        .nodes
        .first()
        .map(|pr| pr.number)
        .ok_or_else(|| VkError::NoPrForBranch {
            branch: branch.into(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserialize_pr_for_branch_response() {
        let json = json!({
            "repository": {
                "pullRequests": {
                    "nodes": [{ "number": 42 }]
                }
            }
        });
        let data: PrForBranchData = serde_json::from_value(json).expect("deserialize");
        assert_eq!(
            data.repository
                .pull_requests
                .nodes
                .first()
                .expect("at least one node")
                .number,
            42
        );
    }

    #[test]
    fn deserialize_pr_for_branch_empty() {
        let json = json!({
            "repository": {
                "pullRequests": {
                    "nodes": []
                }
            }
        });
        let data: PrForBranchData = serde_json::from_value(json).expect("deserialize");
        assert!(data.repository.pull_requests.nodes.is_empty());
    }
}

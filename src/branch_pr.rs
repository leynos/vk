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
#[serde(rename_all = "camelCase")]
struct PrNode {
    number: u64,
    head_repository: Option<HeadRepository>,
}

#[derive(Debug, Deserialize)]
struct HeadRepository {
    owner: Owner,
}

#[derive(Debug, Deserialize)]
struct Owner {
    login: String,
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserialize_pr_for_branch_response() {
        let json = json!({
            "repository": {
                "pullRequests": {
                    "nodes": [{
                        "number": 42,
                        "headRepository": {
                            "owner": { "login": "fork-owner" }
                        }
                    }]
                }
            }
        });
        let data: PrForBranchData = serde_json::from_value(json).expect("deserialize");
        let pr = data
            .repository
            .pull_requests
            .nodes
            .first()
            .expect("at least one node");
        assert_eq!(pr.number, 42);
        assert_eq!(
            pr.head_repository
                .as_ref()
                .expect("head repository")
                .owner
                .login,
            "fork-owner"
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

    #[test]
    fn deserialize_pr_for_branch_null_head_repository() {
        // headRepository can be null for PRs from deleted forks
        let json = json!({
            "repository": {
                "pullRequests": {
                    "nodes": [{
                        "number": 99,
                        "headRepository": null
                    }]
                }
            }
        });
        let data: PrForBranchData = serde_json::from_value(json).expect("deserialize");
        let pr = data
            .repository
            .pull_requests
            .nodes
            .first()
            .expect("at least one node");
        assert_eq!(pr.number, 99);
        assert!(pr.head_repository.is_none());
    }

    #[test]
    fn filter_prs_by_head_owner() {
        let prs = [
            PrNode {
                number: 1,
                head_repository: Some(HeadRepository {
                    owner: Owner {
                        login: "other-owner".to_string(),
                    },
                }),
            },
            PrNode {
                number: 2,
                head_repository: Some(HeadRepository {
                    owner: Owner {
                        login: "target-owner".to_string(),
                    },
                }),
            },
            PrNode {
                number: 3,
                head_repository: None, // Deleted fork
            },
        ];

        // Find PR by head owner
        let matching = prs.iter().find(|pr| {
            pr.head_repository
                .as_ref()
                .is_some_and(|hr| hr.owner.login.eq_ignore_ascii_case("target-owner"))
        });
        assert_eq!(matching.expect("found PR").number, 2);

        // Case-insensitive match
        let matching_case = prs.iter().find(|pr| {
            pr.head_repository
                .as_ref()
                .is_some_and(|hr| hr.owner.login.eq_ignore_ascii_case("TARGET-OWNER"))
        });
        assert_eq!(matching_case.expect("found PR").number, 2);

        // No match for unknown owner
        let no_match = prs.iter().find(|pr| {
            pr.head_repository
                .as_ref()
                .is_some_and(|hr| hr.owner.login.eq_ignore_ascii_case("unknown"))
        });
        assert!(no_match.is_none());
    }
}

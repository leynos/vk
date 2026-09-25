//! Tests for `branch_pr` module.

use super::*;
use serde_json::json;

#[test]
fn deserialize_pr_for_branch_response() {
    let json = json!({
        "repository": {
            "pullRequests": {
                "pageInfo": { "hasNextPage": false, "endCursor": null },
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
                "pageInfo": { "hasNextPage": false, "endCursor": null },
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
                "pageInfo": { "hasNextPage": false, "endCursor": null },
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

    // Find PR by head owner using the helper
    let matching = prs.iter().find(|pr| head_owner_matches(pr, "target-owner"));
    assert_eq!(matching.expect("found PR").number, 2);

    // Case-insensitive match
    let matching_case = prs.iter().find(|pr| head_owner_matches(pr, "TARGET-OWNER"));
    assert_eq!(matching_case.expect("found PR").number, 2);

    // No match for unknown owner
    let no_match = prs.iter().find(|pr| head_owner_matches(pr, "unknown"));
    assert!(no_match.is_none());
}

mod fetch_pr_for_branch_tests;

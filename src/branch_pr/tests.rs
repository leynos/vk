//! Tests for `branch_pr` module.

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

mod fetch_pr_for_branch_tests {
    use super::*;
    use crate::api::RetryConfig;
    use rstest::{fixture, rstest};
    use std::convert::Infallible;
    use std::sync::Arc;
    use third_wheel::hyper::{Body, Response, Server, StatusCode, service::service_fn};
    use tokio::task::JoinHandle;
    use tokio::time::Duration;

    /// RAII guard for mock server cleanup.
    struct MockServer {
        client: GraphQLClient,
        join: JoinHandle<()>,
    }

    impl MockServer {
        fn client(&self) -> &GraphQLClient {
            &self.client
        }
    }

    impl Drop for MockServer {
        fn drop(&mut self) {
            self.join.abort();
        }
    }

    /// Start a mock HTTP server that returns the given JSON body.
    fn start_mock_server(body: String) -> MockServer {
        let body = Arc::new(body);
        let svc = third_wheel::hyper::service::make_service_fn(move |_conn| {
            let body = Arc::clone(&body);
            async move {
                Ok::<_, Infallible>(service_fn(move |_req| {
                    let body = Arc::clone(&body);
                    async move {
                        Ok::<_, Infallible>(
                            Response::builder()
                                .status(StatusCode::OK)
                                .header("Content-Type", "application/json")
                                .body(Body::from(body.as_ref().clone()))
                                .expect("response"),
                        )
                    }
                }))
            }
        });
        let server = Server::bind(&"127.0.0.1:0".parse().expect("addr")).serve(svc);
        let addr = server.local_addr();
        let join = tokio::spawn(async move {
            let _ = server.await;
        });
        let retry = RetryConfig {
            base_delay: Duration::from_millis(1),
            jitter: false,
            ..RetryConfig::default()
        };
        let client =
            GraphQLClient::with_endpoint_retry("token", format!("http://{addr}"), None, retry)
                .expect("client");
        MockServer { client, join }
    }

    #[fixture]
    fn basic_repo() -> RepoInfo {
        RepoInfo {
            owner: "owner".into(),
            name: "repo".into(),
        }
    }

    #[fixture]
    fn upstream_repo() -> RepoInfo {
        RepoInfo {
            owner: "upstream".into(),
            name: "repo".into(),
        }
    }

    /// Node data for building mock PR lookup responses.
    #[derive(Debug)]
    struct TestPrNode {
        number: u64,
        head_owner: Option<&'static str>,
    }

    /// Build a JSON response for the PR-for-branch GraphQL query.
    fn build_pr_lookup_response(nodes: &[TestPrNode]) -> String {
        use serde_json::Value;

        let nodes_json: Vec<Value> = nodes
            .iter()
            .map(|pr| {
                let head_repository = pr
                    .head_owner
                    .map_or(Value::Null, |owner| json!({"owner": {"login": owner}}));
                json!({"number": pr.number, "headRepository": head_repository})
            })
            .collect();
        json!({"data": {"repository": {"pullRequests": {"nodes": nodes_json}}}}).to_string()
    }

    #[rstest]
    #[tokio::test]
    async fn returns_pr_number_on_success(basic_repo: RepoInfo) {
        let body = json!({
            "data": {
                "repository": {
                    "pullRequests": {
                        "nodes": [{
                            "number": 42,
                            "headRepository": {
                                "owner": { "login": "my-fork" }
                            }
                        }]
                    }
                }
            }
        })
        .to_string();
        let server = start_mock_server(body);

        let result = fetch_pr_for_branch(server.client(), &basic_repo, "feature", None).await;

        assert_eq!(result.expect("success"), 42);
    }

    #[rstest]
    #[tokio::test]
    async fn returns_no_pr_for_branch_when_empty(basic_repo: RepoInfo) {
        let body = json!({
            "data": {
                "repository": {
                    "pullRequests": {
                        "nodes": []
                    }
                }
            }
        })
        .to_string();
        let server = start_mock_server(body);

        let result = fetch_pr_for_branch(server.client(), &basic_repo, "orphan", None).await;

        match result {
            Err(VkError::NoPrForBranch { branch }) => {
                assert_eq!(branch.as_ref(), "orphan");
            }
            other => panic!("expected NoPrForBranch, got {other:?}"),
        }
    }

    /// Test cases for PR filtering by head owner.
    ///
    /// Each case specifies:
    /// - A list of PRs (number, `head_owner`)
    /// - The `head_owner` filter to apply
    /// - The expected PR number result
    #[rstest]
    #[case::filters_by_head_owner_when_provided(
        &[(100, Some("other-fork")), (200, Some("my-fork"))],
        Some("my-fork"),
        200
    )]
    #[case::skips_pr_with_null_head_repository(
        &[(100, None), (200, Some("my-fork"))],
        Some("my-fork"),
        200
    )]
    #[case::returns_first_pr_when_head_owner_is_none(
        &[(100, None), (200, Some("my-fork"))],
        None,
        100
    )]
    #[tokio::test]
    async fn head_owner_filtering(
        upstream_repo: RepoInfo,
        #[case] prs: &[(u64, Option<&'static str>)],
        #[case] head_owner: Option<&str>,
        #[case] expected: u64,
    ) {
        let nodes: Vec<_> = prs
            .iter()
            .map(|(number, owner)| TestPrNode {
                number: *number,
                head_owner: *owner,
            })
            .collect();
        let body = build_pr_lookup_response(&nodes);
        let server = start_mock_server(body);

        let result =
            fetch_pr_for_branch(server.client(), &upstream_repo, "feature", head_owner).await;

        assert_eq!(result.expect("success"), expected);
    }

    #[rstest]
    #[tokio::test]
    async fn returns_no_pr_when_head_owner_not_found(upstream_repo: RepoInfo) {
        let body = json!({
            "data": {
                "repository": {
                    "pullRequests": {
                        "nodes": [{
                            "number": 100,
                            "headRepository": {
                                "owner": { "login": "other-fork" }
                            }
                        }]
                    }
                }
            }
        })
        .to_string();
        let server = start_mock_server(body);

        let result = fetch_pr_for_branch(
            server.client(),
            &upstream_repo,
            "feature",
            Some("nonexistent-fork"),
        )
        .await;

        match result {
            Err(VkError::NoPrForBranch { branch }) => {
                assert_eq!(branch.as_ref(), "feature");
            }
            other => panic!("expected NoPrForBranch, got {other:?}"),
        }
    }
}

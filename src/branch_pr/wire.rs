//! GraphQL wire adapter for the branch pull-request lookup.
//!
//! Every transport-facing shape of the lookup lives here: the
//! `#[derive(GraphQLQuery)]` operation, the hand-written response envelopes,
//! the generated-variable construction, and the call into [`GraphQLClient`].
//! Each response is flattened into the parent module's [`PrPage`] before it is
//! returned, so no generated type and no envelope escapes this submodule.

use crate::{GraphQLClient, PageInfo, VkError};
use graphql_client::GraphQLQuery;
use serde::Deserialize;

use super::{CandidatePr, PrPage};

/// Typed `PrForBranchQuery` operation: the paginated PR-by-branch lookup.
///
/// The response is decoded into the hand-written [`PrForBranchData`] via
/// [`GraphQLClient::run_operation_as`] rather than the generated `ResponseData`
/// so the candidate [`CandidatePr::number`] stays `u64` (the schema types it as
/// `Int`, i.e. `i64`). The query itself is still schema-checked at compile
/// time.
///
/// [`GraphQLClient::run_operation_as`]: crate::GraphQLClient::run_operation_as
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/schema.docs.graphql",
    query_path = "graphql/pr_for_branch.graphql",
    response_derives = "Debug, Clone, PartialEq"
)]
pub(super) struct PrForBranchQuery;

/// GraphQL data returned when looking up pull requests for a branch.
#[derive(Debug, Deserialize)]
struct PrForBranchData {
    /// Repository containing the matching pull requests.
    repository: PrForBranchRepository,
}

/// Repository portion of a branch pull-request lookup response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrForBranchRepository {
    /// Pull requests matching the branch query.
    pull_requests: PrConnection,
}

/// Connection containing pull requests returned by GitHub.
#[derive(Debug, Deserialize)]
struct PrConnection {
    /// Pull-request nodes in the connection.
    nodes: Vec<PrNode>,
    /// Pagination metadata for this page.
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
}

/// Pull-request identity and head-repository data.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrNode {
    /// Pull-request number.
    number: u64,
    /// Repository from which the pull request originates, when available.
    head_repository: Option<HeadRepository>,
}

/// Head-repository data for a pull request.
#[derive(Debug, Deserialize)]
struct HeadRepository {
    /// Owner of the head repository.
    owner: Owner,
}

/// GitHub repository-owner information.
#[derive(Debug, Deserialize)]
struct Owner {
    /// GitHub login for the owner.
    login: String,
}

impl PrForBranchData {
    /// Flatten the response envelope into the domain page shape.
    fn into_page(self) -> PrPage {
        let connection = self.repository.pull_requests;
        PrPage {
            prs: connection
                .nodes
                .into_iter()
                .map(PrNode::into_candidate)
                .collect(),
            page_info: connection.page_info,
        }
    }
}

impl PrNode {
    /// Flatten one wire node into a domain candidate.
    ///
    /// A missing head repository (a pull request from a deleted fork) becomes a
    /// candidate with no head owner, matching the former
    /// [`super::CandidatePr::head_owner_matches`] treatment of null nodes.
    fn into_candidate(self) -> CandidatePr {
        CandidatePr {
            number: self.number,
            head_owner: self.head_repository.map(|head| head.owner.login),
        }
    }
}

/// Parameters for one branch pull-request page request.
#[derive(Debug)]
pub(crate) struct PrPageRequest<'a> {
    /// Repository owner login.
    pub(crate) owner: &'a str,
    /// Repository name.
    pub(crate) name: &'a str,
    /// Head branch name to match.
    pub(crate) branch: &'a str,
    /// Cursor after which to fetch the next page; `None` requests the first.
    pub(crate) after: Option<String>,
}

#[cfg(test)]
use mockall::automock;

/// Fetches one page of branch pull-request candidates.
#[cfg_attr(test, automock)]
#[allow(clippy::ref_option, reason = "automock generates &Option")]
#[expect(
    clippy::elidable_lifetime_names,
    reason = "automock requires explicit lifetime for request struct"
)]
pub(crate) trait PrForBranchFetcher {
    /// Fetch the page of candidates following the request's cursor.
    async fn fetch_pr_page<'a>(&self, request: PrPageRequest<'a>) -> Result<PrPage, VkError>;
}

impl PrForBranchFetcher for GraphQLClient {
    #[expect(
        clippy::elidable_lifetime_names,
        reason = "must match the trait's explicit lifetime for automock"
    )]
    async fn fetch_pr_page<'a>(&self, request: PrPageRequest<'a>) -> Result<PrPage, VkError> {
        let variables = pr_for_branch_query::Variables {
            owner: request.owner.to_string(),
            name: request.name.to_string(),
            head_ref: request.branch.to_string(),
            after: request.after,
        };
        let data: PrForBranchData = self
            .run_operation_as::<PrForBranchQuery, PrForBranchData>(variables)
            .await?;
        Ok(data.into_page())
    }
}

#[cfg(test)]
mod tests {
    //! Envelope decoding and wire-to-domain mapping tests.

    use super::*;
    use crate::ref_parser::RepoInfo;
    use serde_json::json;

    /// Build one wire node with an optional head-owner login.
    fn node(number: u64, head_owner: Option<&str>) -> PrNode {
        PrNode {
            number,
            head_repository: head_owner.map(|login| HeadRepository {
                owner: Owner {
                    login: login.to_string(),
                },
            }),
        }
    }

    /// Build a response envelope around the given nodes.
    fn envelope(
        nodes: Vec<PrNode>,
        has_next_page: bool,
        end_cursor: Option<&str>,
    ) -> PrForBranchData {
        PrForBranchData {
            repository: PrForBranchRepository {
                pull_requests: PrConnection {
                    nodes,
                    page_info: PageInfo {
                        has_next_page,
                        end_cursor: end_cursor.map(ToOwned::to_owned),
                    },
                },
            },
        }
    }

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
    fn maps_nodes_and_page_info_into_a_domain_page() {
        let data = envelope(
            vec![node(7, Some("fork-owner")), node(9, None)],
            true,
            Some("page-2"),
        );

        let page = data.into_page();

        assert_eq!(
            page.prs,
            vec![
                CandidatePr {
                    number: 7,
                    head_owner: Some("fork-owner".to_string()),
                },
                CandidatePr {
                    number: 9,
                    head_owner: None,
                },
            ]
        );
        assert!(page.page_info.has_next_page);
        assert_eq!(page.page_info.end_cursor.as_deref(), Some("page-2"));
    }

    #[test]
    fn into_candidate_drops_the_head_repository_envelope() {
        assert_eq!(
            node(11, Some("fork-owner")).into_candidate(),
            CandidatePr {
                number: 11,
                head_owner: Some("fork-owner".to_string()),
            }
        );
        assert_eq!(
            node(12, None).into_candidate(),
            CandidatePr {
                number: 12,
                head_owner: None,
            }
        );
    }

    #[tokio::test]
    async fn fetch_pr_page_sends_the_request_variables() {
        let body = json!({"data": {"repository": {"pullRequests": {
            "nodes": [{"number": 42, "headRepository": {"owner": {"login": "fork-owner"}}}],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}})
        .to_string();
        let crate::test_utils::TestClient {
            client,
            join,
            requests,
            ..
        } = crate::test_utils::start_server(vec![body]);
        let repo = RepoInfo {
            owner: "owner".into(),
            name: "repository".into(),
        };

        let page = client
            .fetch_pr_page(PrPageRequest {
                owner: &repo.owner,
                name: &repo.name,
                branch: "feature",
                after: None,
            })
            .await
            .expect("fetch page");

        assert_eq!(page.prs.len(), 1);
        {
            let requests = requests.lock().expect("lock requests");
            let request = requests.first().expect("one request");
            assert_eq!(
                request.get("operationName"),
                Some(&json!("PrForBranchQuery"))
            );
            assert_eq!(request.pointer("/variables/owner"), Some(&json!("owner")));
            assert_eq!(
                request.pointer("/variables/name"),
                Some(&json!("repository"))
            );
            assert_eq!(
                request.pointer("/variables/headRef"),
                Some(&json!("feature"))
            );
            assert_eq!(
                request.pointer("/variables/after"),
                Some(&serde_json::Value::Null)
            );
        }
        join.abort();
        let _ = join.await;
    }
}

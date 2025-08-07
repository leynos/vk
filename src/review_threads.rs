//! Helpers for fetching pull request review threads from the GitHub API.
//!
//! The module defines GraphQL response structures and helpers to retrieve all
//! unresolved review threads along with their comments.

use serde::Deserialize;
use serde_json::json;

use crate::graphql_queries::{COMMENT_QUERY, THREADS_QUERY};
use crate::ref_parser::RepoInfo;
use crate::{GraphQLClient, VkError, paginate};

#[derive(Debug, Deserialize, Default)]
struct ThreadData {
    repository: Repository,
}

#[derive(Debug, Deserialize, Default)]
struct Repository {
    #[serde(rename = "pullRequest")]
    pull_request: PullRequest,
}

#[derive(Debug, Deserialize, Default)]
struct PullRequest {
    #[serde(rename = "reviewThreads")]
    review_threads: ReviewThreadConnection,
}

#[derive(Debug, Deserialize, Default)]
struct CommentNodeWrapper {
    node: Option<CommentNode>,
}

#[derive(Debug, Deserialize, Default)]
struct CommentNode {
    comments: CommentConnection,
}

/// Connection wrapper around [`ReviewThread`] nodes.
#[derive(Debug, Deserialize, Default)]
struct ReviewThreadConnection {
    nodes: Vec<ReviewThread>,
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
}

/// Details of a single review thread.
#[derive(Debug, Deserialize, Default)]
pub struct ReviewThread {
    pub id: String,
    #[serde(rename = "isResolved")]
    #[allow(
        dead_code,
        reason = "GraphQL query requires this field but it is unused"
    )]
    pub is_resolved: bool,
    pub comments: CommentConnection,
}

/// Collection of comments within a review thread.
#[derive(Debug, Deserialize, Default)]
pub struct CommentConnection {
    pub nodes: Vec<ReviewComment>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

/// A single review comment.
#[derive(Debug, Deserialize, Default)]
pub struct ReviewComment {
    pub body: String,
    #[serde(rename = "diffHunk")]
    pub diff_hunk: String,
    #[serde(rename = "originalPosition")]
    pub original_position: Option<i32>,
    pub position: Option<i32>,
    #[allow(dead_code, reason = "stored for completeness; not displayed yet")]
    pub path: String,
    pub url: String,
    pub author: Option<User>,
}

/// Pagination information returned by GitHub's GraphQL API.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct PageInfo {
    #[serde(rename = "hasNextPage")]
    pub has_next_page: bool,
    #[serde(rename = "endCursor")]
    pub end_cursor: Option<String>,
}

/// Minimal user representation for authorship information.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct User {
    pub login: String,
}

async fn fetch_comment_page(
    client: &GraphQLClient,
    id: &str,
    cursor: Option<String>,
) -> Result<(Vec<ReviewComment>, PageInfo), VkError> {
    let wrapper: CommentNodeWrapper = client
        .run_query(COMMENT_QUERY, json!({ "id": id, "cursor": cursor.clone() }))
        .await?;
    let conn = wrapper
        .node
        .ok_or_else(|| {
            VkError::BadResponse(format!(
                "Missing comment node in response (id: {}, cursor: {})",
                id,
                cursor.as_deref().unwrap_or("None")
            ))
        })?
        .comments;
    Ok((conn.nodes, conn.page_info))
}

async fn fetch_thread_page(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
    cursor: Option<String>,
) -> Result<(Vec<ReviewThread>, PageInfo), VkError> {
    let data: ThreadData = client
        .run_query(
            THREADS_QUERY,
            json!({
                "owner": repo.owner.as_str(),
                "name": repo.name.as_str(),
                "number": number,
                "cursor": cursor,
            }),
        )
        .await?;
    let conn = data.repository.pull_request.review_threads;
    Ok((conn.nodes, conn.page_info))
}

/// Fetch all unresolved review threads for a pull request.
///
/// # Errors
///
/// Returns an error if any API request fails or the response is malformed.
pub async fn fetch_review_threads(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Vec<ReviewThread>, VkError> {
    let mut threads = paginate(|cursor| fetch_thread_page(client, repo, number, cursor)).await?;
    threads.retain(|t| !t.is_resolved);

    for thread in &mut threads {
        let initial = std::mem::replace(
            &mut thread.comments,
            CommentConnection {
                nodes: Vec::new(),
                page_info: PageInfo {
                    has_next_page: false,
                    end_cursor: None,
                },
            },
        );
        let mut comments = initial.nodes;
        if initial.page_info.has_next_page {
            let more = paginate(|c| fetch_comment_page(client, &thread.id, c)).await?;
            comments.extend(more);
        }
        thread.comments = CommentConnection {
            nodes: comments,
            page_info: PageInfo {
                has_next_page: false,
                end_cursor: None,
            },
        };
    }
    Ok(threads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GraphQLClient;
    use crate::ref_parser::RepoInfo;

    #[tokio::test]
    async fn run_query_missing_nodes_reports_path() {
        use third_wheel::hyper::{
            Body, Request, Response, Server, StatusCode,
            service::{make_service_fn, service_fn},
        };

        let body = serde_json::json!({
            "data": {
                "repository": {
                    "pullRequest": {
                        "reviewThreads": {
                            "pageInfo": { "hasNextPage": false, "endCursor": null }
                        }
                    }
                }
            }
        })
        .to_string();

        let make_svc = make_service_fn(move |_conn| {
            let body = body.clone();
            async move {
                Ok::<_, std::convert::Infallible>(service_fn(move |_req: Request<Body>| {
                    let body = body.clone();
                    async move {
                        Ok::<_, std::convert::Infallible>(
                            Response::builder()
                                .status(StatusCode::OK)
                                .header("Content-Type", "application/json")
                                .body(Body::from(body.clone()))
                                .expect("failed to build HTTP response"),
                        )
                    }
                }))
            }
        });

        let server = Server::bind(
            &"127.0.0.1:0"
                .parse()
                .expect("failed to parse server address"),
        )
        .serve(make_svc);
        let addr = server.local_addr();
        let join = tokio::spawn(server);

        let client = GraphQLClient::with_endpoint("token", &format!("http://{addr}"), None)
            .expect("failed to create GraphQL client");

        let repo = RepoInfo {
            owner: "o".into(),
            name: "r".into(),
        };
        let result = fetch_review_threads(&client, &repo, 1).await;
        let err = result.expect_err("expected error");
        let err_msg = format!("{err}");
        assert!(
            err_msg.contains("repository.pullRequest.reviewThreads"),
            "Error should contain full JSON path",
        );
        assert!(
            err_msg.contains("snippet:"),
            "Error should contain JSON snippet",
        );

        join.abort();
        let _ = join.await;
    }
}

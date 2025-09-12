//! Resolve pull request review comments via the GitHub API (GraphQL for resolving, REST for replies).
//!
//! This module posts an optional reply then marks the comment's thread as
//! resolved. The API base URL can be overridden with the `GITHUB_API_URL`
//! environment variable for testing.

use crate::ref_parser::RepoInfo;
use crate::{VkError, api::GraphQLClient};
#[cfg(feature = "unstable-rest-resolve")]
use reqwest::StatusCode;
#[cfg(feature = "unstable-rest-resolve")]
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use serde_json::{Value, json};
#[cfg(feature = "unstable-rest-resolve")]
use std::time::Duration;

const RESOLVE_THREAD_MUTATION: &str = r"
    mutation($id: ID!) {
      resolveReviewThread(input: {threadId: $id}) { clientMutationId }
    }
";

const REVIEW_COMMENTS_PAGE: &str = r"
    query($owner: String!, $name: String!, $pull: Int!, $after: String) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $pull) {
          reviewComments(first: 50, after: $after) {
            pageInfo { endCursor hasNextPage }
            nodes { databaseId pullRequestReviewThread { id } }
          }
        }
      }
    }
";

/// Comment location within a pull request review thread.
#[derive(Copy, Clone)]
pub struct CommentRef<'a> {
    pub repo: &'a RepoInfo,
    pub pull_number: u64,
    pub comment_id: u64,
}
/// Build an authenticated client with GitHub headers.
///
/// # Errors
///
/// Returns [`VkError::RequestContext`] when the client cannot be built.
///
/// # Examples
///
/// ```ignore
/// # use crate::resolve::github_client;
/// let client = github_client("token")?;
/// # Ok::<(), VkError>(())
/// ```
#[cfg(feature = "unstable-rest-resolve")]
fn github_client(token: &str) -> Result<reqwest::Client, VkError> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, "vk".parse().expect("static header"));
    headers.insert(
        AUTHORIZATION,
        format!("Bearer {token}").parse().expect("auth header"),
    );
    headers.insert(
        ACCEPT,
        "application/vnd.github+json"
            .parse()
            .expect("accept header"),
    );
    headers.insert(
        HeaderName::from_static("x-github-api-version"),
        HeaderValue::from_static("2022-11-28"),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| VkError::RequestContext {
            context: "build client".into(),
            source: Box::new(e),
        })
}
/// GitHub REST client configuration.
#[cfg(feature = "unstable-rest-resolve")]
struct RestClient {
    api: String,
    client: reqwest::Client,
}

#[cfg(feature = "unstable-rest-resolve")]
impl RestClient {
    fn new(token: &str) -> Result<Self, VkError> {
        let api = std::env::var("GITHUB_API_URL")
            .unwrap_or_else(|_| "https://api.github.com".into())
            .trim_end_matches('/')
            .to_owned();
        let client = github_client(token)?;
        Ok(Self { api, client })
    }
}

/// Post a reply to a review comment using the REST API.
///
/// # Examples
///
/// ```ignore
/// # use crate::{ref_parser::RepoInfo, resolve::{post_reply, RestClient}, VkError};
/// # async fn run() -> Result<(), VkError> {
/// let repo = RepoInfo { owner: "octocat", name: "hello" };
/// let client = RestClient::new("token")?;
/// post_reply(&client, CommentRef { repo: &repo, pull_number: 1, comment_id: 2 }, "Thanks").await?;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "unstable-rest-resolve")]
async fn post_reply(
    rest: &RestClient,
    reference: CommentRef<'_>,
    body: &str,
) -> Result<(), VkError> {
    let url = format!(
        "{api}/repos/{owner}/{repo}/pulls/comments/{cid}/replies",
        api = rest.api,
        owner = reference.repo.owner,
        repo = reference.repo.name,
        cid = reference.comment_id,
    );

    let response = rest
        .client
        .post(url)
        .json(&json!({ "body": body }))
        .send()
        .await
        .map_err(|e| VkError::RequestContext {
            context: "post reply".into(),
            source: Box::new(e),
        })?;

    if response.status() == StatusCode::NOT_FOUND {
        return Err(VkError::CommentNotFound {
            comment_id: reference.comment_id,
        });
    }

    response
        .error_for_status()
        .map_err(|e| VkError::Request(Box::new(e)))?;
    Ok(())
}

/// Search a page of review comment nodes for a matching comment.
///
/// Returns the thread ID when the comment matches.
///
/// # Examples
///
/// ```ignore
/// # use serde_json::json;
/// let nodes = vec![json!({ "databaseId": 1, "pullRequestReviewThread": { "id": "t" } })];
/// assert_eq!(find_comment_in_page(&nodes, 1), Some("t".into()));
/// ```
fn find_comment_in_page(nodes: &[Value], comment_id: u64) -> Option<String> {
    for node in nodes {
        let Some(id) = node.get("databaseId").and_then(Value::as_u64) else {
            continue;
        };
        if id == comment_id {
            return node
                .get("pullRequestReviewThread")
                .and_then(|t| t.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
    }
    None
}

/// Extract the reviewComments object from a GraphQL response.
///
/// # Errors
///
/// Returns [`VkError::BadResponse`] when expected fields are missing.
///
/// # Examples
///
/// ```ignore
/// # use serde_json::json;
/// let data = json!({"repository": {"pullRequest": {"reviewComments": {}}}});
/// let comments = extract_review_comments(&data)?;
/// # Ok::<(), VkError>(())
/// ```
fn extract_review_comments(data: &Value) -> Result<&Value, VkError> {
    data.get("repository")
        .and_then(|r| r.get("pullRequest"))
        .and_then(|p| p.get("reviewComments"))
        .ok_or_else(|| VkError::BadResponse("missing review comments".into()))
}

/// Look up the GraphQL thread ID for a review comment.
///
/// # Examples
///
/// ```ignore
/// # use crate::{api::GraphQLClient, ref_parser::RepoInfo, resolve::{get_thread_id, CommentRef}, VkError};
/// # async fn run(client: GraphQLClient) -> Result<(), VkError> {
/// let repo = RepoInfo { owner: "o", name: "r" };
/// let id = get_thread_id(&client, CommentRef { repo: &repo, pull_number: 1, comment_id: 42 }).await?;
/// assert!(!id.is_empty());
/// # Ok(())
/// # }
/// ```
async fn get_thread_id(gql: &GraphQLClient, reference: CommentRef<'_>) -> Result<String, VkError> {
    let mut cursor = None;
    loop {
        let data: Value = gql
            .run_query(
                REVIEW_COMMENTS_PAGE,
                json!({
                    "owner": reference.repo.owner,
                    "name": reference.repo.name,
                    "pull": reference.pull_number,
                    "after": cursor,
                }),
            )
            .await?;
        let comments = extract_review_comments(&data)?;
        if let Some(nodes) = comments.get("nodes").and_then(Value::as_array) {
            if let Some(thread_id) = find_comment_in_page(nodes, reference.comment_id) {
                return Ok(thread_id);
            }
            if nodes
                .iter()
                .any(|n| n.get("databaseId").and_then(Value::as_u64) == Some(reference.comment_id))
            {
                return Err(VkError::BadResponse("missing thread id".into()));
            }
        }
        let page = comments
            .get("pageInfo")
            .ok_or_else(|| VkError::BadResponse("missing page info".into()))?;
        let has_next = page
            .get("hasNextPage")
            .and_then(Value::as_bool)
            .ok_or_else(|| VkError::BadResponse("missing hasNextPage".into()))?;
        if !has_next {
            break;
        }
        cursor = Some(
            page.get("endCursor")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| VkError::BadResponse("missing endCursor".into()))?,
        );
    }
    Err(VkError::CommentNotFound {
        comment_id: reference.comment_id,
    })
}

/// Resolve a review thread by ID.
///
/// # Examples
///
/// ```ignore
/// # use crate::{api::GraphQLClient, resolve::resolve_thread, VkError};
/// # async fn run(client: GraphQLClient) -> Result<(), VkError> {
/// resolve_thread(&client, "thread").await?;
/// # Ok(())
/// # }
/// ```
async fn resolve_thread(gql: &GraphQLClient, thread_id: &str) -> Result<(), VkError> {
    gql.run_query::<_, Value>(RESOLVE_THREAD_MUTATION, json!({ "id": thread_id }))
        .await?;
    Ok(())
}
/// Resolve a pull request review comment and optionally post a reply.
///
/// # Errors
///
/// Returns [`VkError::RequestContext`] if an HTTP request fails.
/// Returns [`VkError::CommentNotFound`] if the comment cannot be located.
///
/// # Examples
///
/// ```ignore
/// # use crate::{ref_parser::RepoInfo, resolve::{resolve_comment, CommentRef}, VkError};
/// # async fn run() -> Result<(), VkError> {
/// let repo = RepoInfo { owner: "octocat", name: "hello" };
/// resolve_comment(
///     "token",
///     CommentRef { repo: &repo, pull_number: 1, comment_id: 2 },
///     None,
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn resolve_comment(
    token: &str,
    reference: CommentRef<'_>,
    #[cfg(feature = "unstable-rest-resolve")] message: Option<String>,
) -> Result<(), VkError> {
    #[cfg(feature = "unstable-rest-resolve")]
    if let Some(body) = message {
        let rest = RestClient::new(token)?;
        post_reply(&rest, reference, &body).await?;
    }

    let gql = GraphQLClient::new(token, None)?;
    let thread_id = get_thread_id(&gql, reference).await?;
    resolve_thread(&gql, &thread_id).await?;
    Ok(())
}

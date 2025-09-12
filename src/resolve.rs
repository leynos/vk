//! Resolve pull request review comments via the GitHub API (GraphQL for resolving, REST for replies).
//!
//! This module posts an optional reply then marks the comment's thread as
//! resolved. The API base URL can be overridden with the `GITHUB_API_URL`
//! environment variable for testing.

use crate::ref_parser::RepoInfo;
use crate::{VkError, api::GraphQLClient};
use base64::{Engine as _, engine::general_purpose::STANDARD};
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

const THREAD_ID_QUERY: &str = r"
    query($id: ID!) {
      node(id: $id) {
        ... on PullRequestReviewComment {
          pullRequestReviewThread { id }
        }
      }
    }
";

/// Comment location within a pull request review thread.
#[derive(Copy, Clone)]
pub struct CommentRef<'a> {
    pub repo: &'a RepoInfo,
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
/// post_reply(&client, CommentRef { repo: &repo, comment_id: 2 }, "Thanks").await?;
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

/// Look up the GraphQL thread ID for a review comment.
///
/// # Examples
///
/// ```ignore
/// # use crate::{api::GraphQLClient, resolve::get_thread_id, VkError};
/// # async fn run(client: GraphQLClient) -> Result<(), VkError> {
/// let id = get_thread_id(&client, 42).await?;
/// assert!(!id.is_empty());
/// # Ok(())
/// # }
/// ```
async fn get_thread_id(gql: &GraphQLClient, comment_id: u64) -> Result<String, VkError> {
    let node = STANDARD.encode(format!("PullRequestReviewComment:{comment_id}"));
    let data: Value = gql
        .run_query(THREAD_ID_QUERY, json!({ "id": node }))
        .await?;
    data.get("node")
        .and_then(|n| n.get("pullRequestReviewThread"))
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| VkError::BadResponse("missing thread id".into()))
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
///     CommentRef { repo: &repo, comment_id: 2 },
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
        let repo = reference.repo;
        post_reply(
            &rest,
            CommentRef {
                repo,
                comment_id: reference.comment_id,
            },
            &body,
        )
        .await?;
    }

    let gql = GraphQLClient::new(token, None)?;
    let thread_id = get_thread_id(&gql, reference.comment_id).await?;
    resolve_thread(&gql, &thread_id).await?;
    Ok(())
}

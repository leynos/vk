//! Resolve pull request review comments via the GitHub API (GraphQL for
//! resolving, REST for replies).
//!
//! This module posts an optional reply then marks the comment's thread as
//! resolved. The API base URL can be overridden with the `GITHUB_API_URL`
//! environment variable for testing.

use crate::ref_parser::RepoInfo;
use crate::{api::GraphQLClient, VkError};
#[cfg(feature = "unstable-rest-resolve")]
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, USER_AGENT};
#[cfg(feature = "unstable-rest-resolve")]
use reqwest::StatusCode;
use serde_json::{json, Value};
#[cfg(feature = "unstable-rest-resolve")]
use std::time::Duration;

const RESOLVE_THREAD_MUTATION: &str = r"
    mutation($id: ID!) {
      resolveReviewThread(input: {threadId: $id}) { clientMutationId }
    }
";

const REVIEW_COMMENTS_PAGE: &str = r"
    query($owner: String!, $name: String!, $number: Int!, $after: String) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          reviewComments(first: 100, after: $after) {
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
/// Returns [`VkError::RequestContext`] when the client cannot be built.
///
/// # Examples
///
/// ```ignore
/// # use crate::resolve::github_client;
/// use std::time::Duration;
/// let client = github_client("token", Duration::from_secs(10), Duration::from_secs(5))?;
/// # Ok::<(), VkError>(())
/// ```
#[cfg(feature = "unstable-rest-resolve")]
fn github_client(
    token: &str,
    timeout: Duration,
    connect_timeout: Duration,
) -> Result<reqwest::Client, VkError> {
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
        .timeout(timeout)
        .connect_timeout(connect_timeout)
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
    fn new(token: &str, timeout: Duration, connect_timeout: Duration) -> Result<Self, VkError> {
        let api = std::env::var("GITHUB_API_URL")
            .unwrap_or_else(|_| "https://api.github.com".into())
            .trim_end_matches('/')
            .to_owned();
        let client = github_client(token, timeout, connect_timeout)?;
        Ok(Self { api, client })
    }
}

/// Post a reply to a review comment using the REST API.
#[cfg(feature = "unstable-rest-resolve")]
async fn post_reply(
    rest: &RestClient,
    reference: CommentRef<'_>,
    body: &str,
) -> Result<(), VkError> {
    let body = body.trim();
    if body.is_empty() {
        // Avoid GitHub 422s by skipping empty replies
        return Ok(());
    }

    let url = format!(
        "{}/repos/{}/{}/pulls/{}/comments/{}/replies",
        rest.api, reference.repo.owner, reference.repo.name, reference.pull_number, reference.comment_id
    );
    let res = rest
        .client
        .post(url)
        .json(&json!({ "body": body }))
        .send()
        .await
        .map_err(|e| VkError::RequestContext {
            context: "post reply".into(),
            source: Box::new(e),
        })?;
    match res.status() {
        StatusCode::CREATED | StatusCode::OK => Ok(()),
        StatusCode::NOT_FOUND => Err(VkError::CommentNotFound {
            comment_id: reference.comment_id,
        }),
        code => Err(VkError::RequestContext {
            context: format!("post reply status {code}").into(),
            source: Box::new(
                res.error_for_status()
                    .err()
                    .unwrap_or_else(|| reqwest::Error::new(reqwest::ErrorKind::Status, "status")),
            ),
        }),
    }
}

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

fn extract_review_comments(data: &Value) -> Result<&Value, VkError> {
    data.get("repository")
        .and_then(|r| r.get("pullRequest"))
        .and_then(|p| p.get("reviewComments"))
        .ok_or_else(|| VkError::BadResponse("missing review comments".into()))
}

fn process_comments_page(comments: &Value, comment_id: u64) -> Result<Option<String>, VkError> {
    if let Some(nodes) = comments.get("nodes").and_then(Value::as_array) {
        if let Some(thread_id) = find_comment_in_page(nodes, comment_id) {
            return Ok(Some(thread_id));
        }
        if nodes
            .iter()
            .any(|n| n.get("databaseId").and_then(Value::as_u64) == Some(comment_id))
        {
            return Err(VkError::BadResponse("missing thread id".into()));
        }
    }
    Ok(None)
}

fn get_page_info(comments: &Value) -> Result<(bool, Option<String>), VkError> {
    let page = comments
        .get("pageInfo")
        .ok_or_else(|| VkError::BadResponse("missing page info".into()))?;
    let has_next = page
        .get("hasNextPage")
        .and_then(Value::as_bool)
        .ok_or_else(|| VkError::BadResponse("missing hasNextPage".into()))?;
    let cursor = if has_next {
        Some(
            page
                .get("endCursor")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| VkError::BadResponse("missing endCursor".into()))?,
        )
    } else {
        None
    };
    Ok((has_next, cursor))
}

async fn get_thread_id(gql: &GraphQLClient, reference: CommentRef<'_>) -> Result<String, VkError> {
    let mut cursor = None;
    loop {
        let data: Value = gql
            .run_query(
                REVIEW_COMMENTS_PAGE,
                json!({
                    "owner": reference.repo.owner,
                    "name": reference.repo.name,
                    "number": reference.pull_number,
                    "after": cursor,
                }),
            )
            .await?;
        let comments = extract_review_comments(&data)?;
        if let Some(id) = process_comments_page(comments, reference.comment_id)? {
            return Ok(id);
        }
        let (has_next, next) = get_page_info(comments)?;
        if !has_next {
            break;
        }
        cursor = next;
    }
    Err(VkError::CommentNotFound {
        comment_id: reference.comment_id,
    })
}

async fn resolve_thread(gql: &GraphQLClient, thread_id: &str) -> Result<(), VkError> {
    gql
        .run_query::<_, Value>(RESOLVE_THREAD_MUTATION, json!({ "id": thread_id }))
        .await?;
    Ok(())
}

/// Resolve a pull request review comment and optionally post a reply.
///
/// Returns [`VkError::RequestContext`] if an HTTP request fails.
/// Returns [`VkError::CommentNotFound`] if the comment cannot be located.
///
/// # Examples
///
/// ```ignore
/// # use crate::{ref_parser::RepoInfo, resolve::{resolve_comment, CommentRef}, VkError};
/// use std::time::Duration;
/// # async fn run() -> Result<(), VkError> {
/// let repo = RepoInfo { owner: "octocat", name: "hello" };
/// resolve_comment(
///     "token",
///     CommentRef { repo: &repo, pull_number: 1, comment_id: 2 },
///     None,
///     Duration::from_secs(10),
///     Duration::from_secs(5),
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn resolve_comment(
    token: &str,
    reference: CommentRef<'_>,
    #[cfg(feature = "unstable-rest-resolve")] message: Option<String>,
    #[cfg(feature = "unstable-rest-resolve")] timeout: Duration,
    #[cfg(feature = "unstable-rest-resolve")] connect_timeout: Duration,
) -> Result<(), VkError> {
    #[cfg(feature = "unstable-rest-resolve")]
    if let Some(body) = message.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        let rest = RestClient::new(token, timeout, connect_timeout)?;
        post_reply(&rest, reference, body).await?;
    }

    let gql = GraphQLClient::new(token, None)?;
    let thread_id = get_thread_id(&gql, reference).await?;
    resolve_thread(&gql, &thread_id).await?;
    Ok(())
}


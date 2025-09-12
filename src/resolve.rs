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

pub async fn resolve_comment(
    token: &str,
    reference: CommentRef<'_>,
    #[cfg(feature = "unstable-rest-resolve")] message: Option<String>,
) -> Result<(), VkError> {
    #[cfg(feature = "unstable-rest-resolve")]
    if let Some(body) = message.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        let rest = RestClient::new(token)?;
        post_reply(&rest, reference, body).await?;
    }

    let gql = GraphQLClient::new(token, None)?;
    let thread_id = get_thread_id(&gql, reference).await?;
    resolve_thread(&gql, &thread_id).await?;
    Ok(())
}

<<<<<<< HEAD
/// Fetch the global node identifier for a review comment via the REST API.
///
/// This is used as a fallback if the GraphQL node encoding changes.
///
/// # Errors
///
/// Returns [`VkError::RequestContext`] when the request fails or
/// [`VkError::BadResponse`] if the `node_id` field is absent.
///
/// # Examples
///
/// ```no_run
/// use crate::ref_parser::RepoInfo;
/// async fn example(token: &str, repo: &RepoInfo) -> Result<(), crate::VkError> {
///     let _ = fetch_comment_node_id(token, repo, 1).await?;
///     Ok(())
/// }
/// ```
#[cfg(feature = "unstable-rest-resolve")]
async fn fetch_comment_node_id(
    token: &str,
    repo: &RepoInfo,
    comment_id: u64,
) -> Result<String, VkError> {
    let api = std::env::var("GITHUB_API_URL")
        .unwrap_or_else(|_| "https://api.github.com".into())
        .trim_end_matches('/')
        .to_owned();
    let client = github_client(token)?;
    let url = format!(
        "{api}/repos/{owner}/{repo}/pulls/comments/{comment_id}",
        owner = repo.owner,
        repo = repo.name,
        comment_id = comment_id,
    );
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| VkError::RequestContext {
            context: "fetch comment".into(),
            source: Box::new(e),
        })?;
    let status = resp.status();
    if !status.is_success() {
        let err = resp.error_for_status_ref().expect_err("status error");
        let text = resp.text().await.unwrap_or_default();
        let snippet: String = text.chars().take(512).collect();
        return Err(VkError::RequestContext {
            context: format!("fetch comment status {status} body {snippet}").into(),
            source: Box::new(err),
        });
    }
    let comment = resp
        .json::<Value>()
        .await
        .map_err(|e| VkError::RequestContext {
            context: "parse comment".into(),
            source: Box::new(e),
        })?;
    comment
        .get("node_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| VkError::BadResponse("missing comment node id".into()))
}

/// Resolve a pull request review comment and optionally post a reply.
///
/// # Errors
///
/// Returns [`VkError::RequestContext`] if an HTTP request fails.
pub async fn resolve_comment(
    token: &str,
    reference: CommentRef<'_>,
    #[cfg(feature = "unstable-rest-resolve")] message: Option<String>,
) -> Result<(), VkError> {
    let comment_id = reference.comment_id;
    #[cfg(feature = "unstable-rest-resolve")]
    let (repo, pull_number) = (reference.repo, reference.pull_number);

    #[cfg(feature = "unstable-rest-resolve")]
    if let Some(body) = message {
||||||| parent of 6f0fae8 (Extract helpers for review comment pagination)
/// Resolve a pull request review comment and optionally post a reply.
///
/// # Errors
///
/// Returns [`VkError::RequestContext`] if an HTTP request fails.
pub async fn resolve_comment(
    token: &str,
    reference: CommentRef<'_>,
    #[cfg(feature = "unstable-rest-resolve")] message: Option<String>,
) -> Result<(), VkError> {
    let comment_id = reference.comment_id;
    #[cfg(feature = "unstable-rest-resolve")]
    let (repo, pull_number) = (reference.repo, reference.pull_number);

    #[cfg(feature = "unstable-rest-resolve")]
    if let Some(body) = message {
=======
#[cfg(feature = "unstable-rest-resolve")]
impl RestClient {
    fn new(token: &str) -> Result<Self, VkError> {
>>>>>>> 6f0fae8 (Extract helpers for review comment pagination)
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
    let body = body.trim();
    if body.is_empty() {
        // Avoid GitHub 422s by skipping empty replies
        return Ok(());
    }

<<<<<<< HEAD
    let gql = GraphQLClient::new(token, None)?;
    // GitHub encodes node identifiers as base64 "<Type>:<id>" values.
    // If this format changes, the lookup below may fail and we fall back to REST.
    let comment_node = STANDARD.encode(format!("PullRequestReviewComment:{comment_id}"));
    let lookup = gql
        .run_query::<_, Value>(THREAD_ID_QUERY, json!({ "id": &comment_node }))
        .await;

    #[cfg(feature = "unstable-rest-resolve")]
    #[expect(
        clippy::single_match_else,
        reason = "fallback to REST on lookup failure"
    )]
    let thread_id = match lookup
        .as_ref()
        .ok()
        .and_then(|data| thread_id_from_lookup(data).map(ToOwned::to_owned))
    {
        Some(id) => id,
        None => {
            if let Err(e) = &lookup {
                warn!("GraphQL thread ID lookup failed: {e}");
            } else {
                warn!("GraphQL thread ID lookup missing thread id");
            }
            let node_id = fetch_comment_node_id(token, reference.repo, comment_id).await?;
            let lookup = gql
                .run_query::<_, Value>(THREAD_ID_QUERY, json!({ "id": node_id }))
                .await?;
            thread_id_from_lookup(&lookup)
                .ok_or_else(|| VkError::BadResponse("missing thread id".into()))?
                .to_owned()
        }
    };

    #[cfg(not(feature = "unstable-rest-resolve"))]
    let thread_id = {
        let data = lookup?;
        thread_id_from_lookup(&data)
            .ok_or_else(|| VkError::BadResponse("missing thread id".into()))?
            .to_owned()
    };
    let vars = json!({ "id": thread_id });
    gql.run_query::<_, Value>(RESOLVE_THREAD_MUTATION, vars)
||||||| parent of 6f0fae8 (Extract helpers for review comment pagination)
    let gql = GraphQLClient::new(token, None)?;
    let comment_node = STANDARD.encode(format!("PullRequestReviewComment:{comment_id}"));
    let lookup = gql
        .run_query::<_, Value>(THREAD_ID_QUERY, json!({ "id": comment_node }))
        .await?;
    let thread_id = lookup
        .get("node")
        .and_then(|n| n.get("pullRequestReviewThread"))
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
        .ok_or_else(|| VkError::BadResponse("missing thread id".into()))?;
    let vars = json!({ "id": thread_id });
    gql.run_query::<_, Value>(RESOLVE_THREAD_MUTATION, vars)
=======
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
    if response.status() != StatusCode::CREATED {
        return response
            .error_for_status()
            .map(|_| ())
            .map_err(|e| VkError::Request(Box::new(e)));
    }
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

/// Process a page of review comments, searching for the target comment.
///
/// Returns the thread ID when found or `Ok(None)` when absent.
///
/// # Errors
///
/// Returns [`VkError::BadResponse`] when the thread ID is missing for a matching comment.
///
/// # Examples
///
/// ```ignore
/// # use serde_json::json;
/// let comments = json!({"nodes": [{"databaseId": 1, "pullRequestReviewThread": {"id": "t"}}]});
/// assert_eq!(process_comments_page(&comments, 1).unwrap(), Some("t".into()));
/// ```
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

/// Extract pagination info from a review comments page.
///
/// Returns whether another page exists and the cursor to fetch it.
///
/// # Errors
///
/// Returns [`VkError::BadResponse`] when pagination fields are missing.
///
/// # Examples
///
/// ```ignore
/// # use serde_json::json;
/// let comments = json!({"pageInfo": {"hasNextPage": true, "endCursor": "c"}});
/// assert_eq!(get_page_info(&comments).unwrap(), (true, Some("c".into())));
/// ```
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
            page.get("endCursor")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| VkError::BadResponse("missing endCursor".into()))?,
        )
    } else {
        None
    };
    Ok((has_next, cursor))
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
>>>>>>> 6f0fae8 (Extract helpers for review comment pagination)
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
    if let Some(body) = message.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        let rest = RestClient::new(token)?;
        post_reply(&rest, reference, body).await?;
    }

    let gql = GraphQLClient::new(token, None)?;
    let thread_id = get_thread_id(&gql, reference).await?;
    resolve_thread(&gql, &thread_id).await?;
    Ok(())
}

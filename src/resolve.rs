//! Resolve pull request review comments via the GitHub API (GraphQL for resolving, REST for replies).
//!
//! This module posts an optional reply then marks the comment's thread as
//! resolved. The API base URL can be overridden with the `GITHUB_API_URL`
//! environment variable for testing.

use crate::ref_parser::RepoInfo;
use crate::{VkError, api::GraphQLClient};
use base64::{Engine as _, engine::general_purpose::STANDARD};
#[cfg(feature = "unstable-rest-resolve")]
use log::warn;
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

/// Extract the thread identifier from a GraphQL lookup.
///
/// # Examples
///
/// ```
/// use serde_json::json;
/// let data = json!({"node": {"pullRequestReviewThread": {"id": "T"}}});
/// assert_eq!(thread_id_from_lookup(&data), Some("T"));
/// ```
fn thread_id_from_lookup(lookup: &Value) -> Option<&str> {
    lookup
        .get("node")
        .and_then(|n| n.get("pullRequestReviewThread"))
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
}

/// Comment location within a pull request review thread.
#[derive(Copy, Clone)]
#[cfg_attr(
    not(feature = "unstable-rest-resolve"),
    expect(dead_code, reason = "unused without unstable-rest-resolve")
)]
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
    if !resp.status().is_success() {
        let status = resp.status();
        let err = resp.error_for_status_ref().expect_err("status error");
        return Err(VkError::RequestContext {
            context: format!("fetch comment status {status}").into(),
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
        let api = std::env::var("GITHUB_API_URL")
            .unwrap_or_else(|_| "https://api.github.com".into())
            .trim_end_matches('/')
            .to_owned();
        let client = github_client(token)?;
        let resp = client
            .post(format!(
                "{api}/repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies",
                owner = repo.owner,
                repo = repo.name,
                pull_number = pull_number,
            ))
            .json(&json!({ "body": body }))
            .send()
            .await
            .map_err(|e| VkError::RequestContext {
                context: "post reply".into(),
                source: Box::new(e),
            })?;
        if resp.status() != StatusCode::NOT_FOUND {
            resp.error_for_status()
                .map_err(|e| VkError::Request(Box::new(e)))?;
        }
    }

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
        .await?;
    Ok(())
}

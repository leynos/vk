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
        .await?;
    Ok(())
}

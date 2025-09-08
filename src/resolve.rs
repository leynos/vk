//! Resolve pull request review comments via the GitHub API (GraphQL for resolving, REST for replies).
//!
//! This module posts an optional reply then marks the comment's thread as
//! resolved. The API base URL can be overridden with the `GITHUB_API_URL`
//! environment variable for testing.

use crate::ref_parser::RepoInfo;
use crate::{VkError, api::GraphQLClient};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use serde_json::{Value, json};
use std::{env, time::Duration};

const RESOLVE_THREAD_MUTATION: &str = r"
    mutation($id: ID!) {
      resolveReviewThread(input: {threadId: $id}) { clientMutationId }
    }
";

/// Build an authenticated client with GitHub headers.
///
/// # Errors
///
/// Returns [`VkError::RequestContext`] when the client cannot be built.
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
    repo: &RepoInfo,
    pull_number: u64,
    comment_id: u64,
    message: Option<String>,
) -> Result<(), VkError> {
    let api = env::var("GITHUB_API_URL")
        .unwrap_or_else(|_| "https://api.github.com".into())
        .trim_end_matches('/')
        .to_owned();
    let client = github_client(token)?;
    #[cfg(feature = "unstable-rest-resolve")]
    if let Some(body) = message {
        let resp = client
            .post(format!(
                "{api}/repos/{owner}/{repo}/pulls/{pull_number}/comments/{comment_id}/replies",
                owner = repo.owner,
                repo = repo.name,
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
    let thread_id = STANDARD.encode(format!("PullRequestReviewThread:{comment_id}"));
    let vars = json!({ "id": thread_id });
    gql.run_query::<_, Value>(RESOLVE_THREAD_MUTATION, vars)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::POST, MockServer};

    #[tokio::test]
    async fn resolve_comment_sends_requests() {
        let server = MockServer::start();
        let reply = server.mock(|when, then| {
            when.method(POST)
                .path("/repos/o/r/pulls/2/comments/1/replies")
                .header("accept", "application/vnd.github+json")
                .header("x-github-api-version", "2022-11-28");
            then.status(200);
        });
        let resolve = server.mock(|when, then| {
            when.method(POST).path("/graphql");
            then.status(200)
                .json_body(json!({"data": {"resolveReviewThread": {"clientMutationId": null}}}));
        });
        crate::test_utils::set_var("GITHUB_API_URL", server.url(""));
        crate::test_utils::set_var("GITHUB_GRAPHQL_URL", server.url("/graphql"));
        let repo = RepoInfo {
            owner: "o".into(),
            name: "r".into(),
        };
        resolve_comment("t", &repo, 2, 1, Some("done".into()))
            .await
            .expect("resolve comment");
        reply.assert();
        resolve.assert();
        crate::test_utils::remove_var("GITHUB_API_URL");
    }
}

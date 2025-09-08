//! Resolve pull request review comments via the GitHub REST API.
//!
//! This module posts an optional reply then marks the comment's thread as
//! resolved. The API base URL can be overridden with the `GITHUB_API_URL`
//! environment variable for testing.

use crate::VkError;
use crate::ref_parser::RepoInfo;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, USER_AGENT};
use serde_json::json;
use std::{env, future::Future, pin::Pin};

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
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| VkError::RequestContext {
            context: "build client".into(),
            source: Box::new(e),
        })
}

trait SendReq {
    fn send_req(
        self,
        ctx: &'static str,
    ) -> Pin<Box<dyn Future<Output = Result<(), VkError>> + Send>>;
}

impl SendReq for reqwest::RequestBuilder {
    fn send_req(
        self,
        ctx: &'static str,
    ) -> Pin<Box<dyn Future<Output = Result<(), VkError>> + Send>> {
        Box::pin(async move {
            self.send()
                .await
                .map_err(|e| VkError::RequestContext {
                    context: ctx.into(),
                    source: Box::new(e),
                })?
                .error_for_status()
                .map_err(|e| VkError::Request(Box::new(e)))?;
            Ok(())
        })
    }
}

/// Resolve a pull request review comment and optionally post a reply.
///
/// # Errors
///
/// Returns [`VkError::RequestContext`] if an HTTP request fails.
pub async fn resolve_comment(
    token: &str,
    repo: &RepoInfo,
    comment_id: u64,
    message: Option<String>,
) -> Result<(), VkError> {
    let api = env::var("GITHUB_API_URL").unwrap_or_else(|_| "https://api.github.com".into());
    let client = github_client(token)?;
    let base = format!(
        "{api}/repos/{owner}/{repo}/pulls/comments/{comment}",
        owner = repo.owner,
        repo = repo.name,
        comment = comment_id
    );

    if let Some(body) = message {
        client
            .post(format!("{base}/replies"))
            .json(&json!({ "body": body }))
            .send_req("post reply")
            .await?;
    }

    client
        .put(format!("{base}/resolve"))
        .send_req("resolve comment")
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::{Method::POST, Method::PUT, MockServer};

    #[tokio::test]
    async fn resolve_comment_sends_requests() {
        let server = MockServer::start();
        let reply = server.mock(|when, then| {
            when.method(POST)
                .path("/repos/o/r/pulls/comments/1/replies");
            then.status(200);
        });
        let resolve = server.mock(|when, then| {
            when.method(PUT).path("/repos/o/r/pulls/comments/1/resolve");
            then.status(200);
        });

        crate::test_utils::set_var("GITHUB_API_URL", server.url(""));
        let repo = RepoInfo {
            owner: "o".into(),
            name: "r".into(),
        };
        resolve_comment("t", &repo, 1, Some("done".into()))
            .await
            .expect("resolve comment");
        reply.assert();
        resolve.assert();
        crate::test_utils::remove_var("GITHUB_API_URL");
    }
}

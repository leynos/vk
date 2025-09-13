//! REST helpers for replying to review comments.

use super::CommentRef;
use crate::VkError;
use log::warn;
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use serde_json::json;
use std::time::Duration;

/// Build an authenticated client with GitHub headers.
///
/// Returns [`VkError::RequestContext`] when the client cannot be built.
///
/// # Examples
///
/// ```ignore
/// # use crate::resolve::rest::github_client;
/// use std::time::Duration;
/// let client = github_client("token", Duration::from_secs(10), Duration::from_secs(5))?;
/// # Ok::<(), VkError>(())
/// ```
pub(crate) fn github_client(
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
pub(crate) struct RestClient {
    api: String,
    client: reqwest::Client,
}

impl RestClient {
    pub(crate) fn new(
        token: &str,
        timeout: Duration,
        connect_timeout: Duration,
    ) -> Result<Self, VkError> {
        let api = std::env::var("GITHUB_API_URL")
            .unwrap_or_else(|_| "https://api.github.com".into())
            .trim_end_matches('/')
            .to_owned();
        let client = github_client(token, timeout, connect_timeout)?;
        Ok(Self { api, client })
    }
}

/// Post a reply to a review comment using the REST API.
pub(crate) async fn post_reply(
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
        rest.api,
        reference.repo.owner,
        reference.repo.name,
        reference.pull_number,
        reference.comment_id
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
    if res.status() == StatusCode::NOT_FOUND {
        warn!(
            "reply target not found: {}/{} comment {} in PR #{}",
            reference.repo.owner, reference.repo.name, reference.comment_id, reference.pull_number
        );
        // Treat missing original comment as non-fatal: continue to resolve.
        return Ok(());
    }
    res.error_for_status()
        .map(|_| ())
        .map_err(|e| VkError::Request(Box::new(e)))
}

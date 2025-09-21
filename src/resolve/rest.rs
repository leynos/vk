//! REST helpers for replying to review comments.

use super::CommentRef;
use crate::{VkError, boxed::BoxedStr, environment};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use reqwest::{StatusCode, Url};
use serde_json::json;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tracing::warn;

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
    let parse_value = |value: &str, context: &str| -> Result<HeaderValue, VkError> {
        HeaderValue::from_str(value).map_err(|e| VkError::RequestContext {
            context: context.boxed(),
            source: e.into(),
        })
    };
    headers.insert(USER_AGENT, parse_value("vk", "build user agent header")?);
    let auth_header = format!("Bearer {token}");
    headers.insert(
        AUTHORIZATION,
        parse_value(&auth_header, "build authorization header")?,
    );
    headers.insert(
        ACCEPT,
        parse_value("application/vnd.github+json", "build accept header")?,
    );
    let api_version_header =
        HeaderName::from_bytes(b"x-github-api-version").map_err(|e| VkError::RequestContext {
            context: "build x-github-api-version header name".boxed(),
            source: e.into(),
        })?;
    headers.insert(
        api_version_header,
        parse_value("2022-11-28", "build x-github-api-version header value")?,
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
    api: Url,
    client: reqwest::Client,
    #[cfg(test)]
    request_count: AtomicUsize,
}

impl RestClient {
    /// Create a REST client targeting the provided `api` base URL.
    /// Falls back to `GITHUB_API_URL` or the public GitHub endpoint when `api` is `None`.
    pub(crate) fn new(
        token: &str,
        api: Option<&str>,
        timeout: Duration,
        connect_timeout: Duration,
    ) -> Result<Self, VkError> {
        let mut base = api
            .map(str::to_owned)
            .or_else(|| environment::var("GITHUB_API_URL").ok())
            .unwrap_or_else(|| "https://api.github.com".into());
        while base.ends_with('/') {
            base.pop();
        }
        let mut api = Url::parse(&base).map_err(|e| VkError::RequestContext {
            context: format!("parse API base URL from {base}").boxed(),
            source: Box::new(e),
        })?;
        let normalised_path = match api.path().trim_end_matches('/') {
            "" => "/".to_owned(),
            path => format!("{path}/"),
        };
        api.set_path(&normalised_path);
        let client = github_client(token, timeout, connect_timeout)?;
        Ok(Self {
            api,
            client,
            #[cfg(test)]
            request_count: AtomicUsize::new(0),
        })
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

    let path = format!(
        "repos/{}/{}/pulls/{}/comments/{}/replies",
        reference.repo.owner, reference.repo.name, reference.pull_number, reference.comment_id
    );
    let url = rest.api.join(&path).map_err(|e| VkError::RequestContext {
        context: format!(
            "build reply URL for comment {} in repo {}/{}",
            reference.comment_id, reference.repo.owner, reference.repo.name
        )
        .boxed(),
        source: Box::new(e),
    })?;
    #[cfg(test)]
    rest.request_count.fetch_add(1, Ordering::SeqCst);
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
            "reply target not found (url={}): {}/{} comment {} in PR #{}",
            res.url(),
            reference.repo.owner,
            reference.repo.name,
            reference.comment_id,
            reference.pull_number
        );
        // Treat missing original comment as non-fatal: continue to resolve.
        return Ok(());
    }
    res.error_for_status()
        .map(|_| ())
        .map_err(|e| VkError::Request(Box::new(e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ref_parser::RepoInfo;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    #[tokio::test]
    async fn skips_whitespace_reply_without_request() {
        let rest = RestClient::new(
            "token",
            Some("https://example.test"),
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .expect("rest client");
        let repo = RepoInfo {
            owner: "octocat".into(),
            name: "hello-world".into(),
        };
        let reference = CommentRef {
            repo: &repo,
            pull_number: 1,
            comment_id: 42,
        };
        post_reply(&rest, reference, "   ")
            .await
            .expect("skip whitespace reply");
        assert_eq!(rest.request_count.load(Ordering::SeqCst), 0);
    }
}

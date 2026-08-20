//! REST helpers for replying to review comments.
//!
//! The reply path is served by [`octocrab`], whose builder supplies the
//! authentication and base-URI plumbing previously hand-rolled on `reqwest`.
//! Only the raw `_post` route is used, so status-code handling stays under
//! `vk`'s control: a 404 is non-fatal (warn and continue), and any other
//! non-2xx status is fatal.

use super::{
    CommentRef,
    rest_invariants::{ReplyStatus, classify_reply_status, normalize_api_base_url},
    rest_metrics::{ReplyAttemptOutcome, record_reply_attempt},
};
use crate::{VkError, boxed::BoxedStr};
use http::header::{ACCEPT, HeaderName};
use octocrab::Octocrab;
use serde_json::json;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tracing::warn;
use vk::environment;

/// GitHub REST client configuration.
///
/// Wraps an [`Octocrab`] instance built from the resolved base URI and the
/// caller's authentication token.
pub(crate) struct RestClient {
    client: Octocrab,
    timeout: Duration,
    #[cfg(test)]
    request_count: AtomicUsize,
}

impl RestClient {
    /// Create a REST client targeting the provided `api` base URL.
    ///
    /// The base URI is resolved in order: the explicit `api` parameter, then the
    /// `GITHUB_API_URL` environment variable, then the public GitHub endpoint.
    /// Plain `http://` loopback addresses are accepted so tests can redirect the
    /// client at a local stub server.
    ///
    /// `personal_token` is only set when `token` is non-empty, preserving
    /// anonymous access. The `connect_timeout` maps directly onto octocrab's
    /// connect timeout. The read and write timeouts are configured on octocrab,
    /// and the original total-request deadline is retained for the complete
    /// reply operation.
    ///
    /// Returns [`VkError::RequestContext`] when the base URI cannot be parsed or
    /// the client cannot be built.
    pub(crate) fn new(
        token: &str,
        api: Option<&str>,
        timeout: Duration,
        connect_timeout: Duration,
    ) -> Result<Self, VkError> {
        let base = normalize_api_base_url(
            api.map(str::to_owned)
                .or_else(|| environment::var("GITHUB_API_URL").ok())
                .unwrap_or_else(|| "https://api.github.com".into()),
        );
        let mut builder =
            Octocrab::builder()
                .base_uri(base.as_str())
                .map_err(|e| VkError::RequestContext {
                    context: format!("parse API base URL from {base}").boxed(),
                    source: Box::new(e),
                })?;
        if !token.is_empty() {
            builder = builder.personal_token(token.to_owned());
        }
        // Restore the behaviourally meaningful headers the reqwest client
        // sent: the pinned REST API version and the GitHub JSON media type.
        // octocrab's default `User-Agent: octocrab` is left alone (decision
        // recorded in the ExecPlan).
        let client = builder
            .add_header(
                HeaderName::from_static("x-github-api-version"),
                "2022-11-28".to_owned(),
            )
            .add_header(ACCEPT, "application/vnd.github+json".to_owned())
            .set_connect_timeout(Some(connect_timeout))
            .set_read_timeout(Some(timeout))
            .set_write_timeout(Some(timeout))
            .build()
            .map_err(|e| VkError::RequestContext {
                context: "build client".boxed(),
                source: Box::new(e),
            })?;
        Ok(Self {
            client,
            timeout,
            #[cfg(test)]
            request_count: AtomicUsize::new(0),
        })
    }
}

/// Post a reply to a review comment using the REST API.
///
/// A 404 response is treated as non-fatal: the original comment is assumed to
/// have gone away, so the caller continues to resolve the thread. Any other
/// non-2xx status is mapped to [`VkError::RequestContext`], whose message names
/// the reply route and the failing status.
pub(crate) async fn post_reply(
    rest: &RestClient,
    reference: CommentRef<'_>,
    body: &str,
) -> Result<(), VkError> {
    let body = body.trim();
    if body.is_empty() {
        // Avoid GitHub 422s by skipping empty replies.
        return Ok(());
    }

    let route = format!(
        "/repos/{}/{}/pulls/{}/comments/{}/replies",
        reference.repo.owner, reference.repo.name, reference.pull_number, reference.comment_id
    );
    #[cfg(test)]
    rest.request_count.fetch_add(1, Ordering::SeqCst);
    let started_at = Instant::now();
    let response = tokio::time::timeout(
        rest.timeout,
        rest.client
            ._post(route.as_str(), Some(&json!({ "body": body }))),
    )
    .await;
    let response = match response {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            record_reply_attempt(started_at, ReplyAttemptOutcome::TransportFailure);
            return Err(VkError::RequestContext {
                context: "post reply".boxed(),
                source: Box::new(error),
            });
        }
        Err(error) => {
            record_reply_attempt(started_at, ReplyAttemptOutcome::Timeout);
            return Err(VkError::RequestContext {
                context: format!("post reply to {route}").boxed(),
                source: Box::new(error),
            });
        }
    };
    let status = response.status();
    match classify_reply_status(status) {
        ReplyStatus::NotFound => {
            record_reply_attempt(
                started_at,
                ReplyAttemptOutcome::Response {
                    status,
                    result: ReplyStatus::NotFound,
                },
            );
            warn!(
                "reply target not found (route={route}): {}/{} comment {} in PR #{}",
                reference.repo.owner,
                reference.repo.name,
                reference.comment_id,
                reference.pull_number
            );
            // Treat missing original comment as non-fatal: continue to resolve.
            Ok(())
        }
        ReplyStatus::Success => {
            record_reply_attempt(
                started_at,
                ReplyAttemptOutcome::Response {
                    status,
                    result: ReplyStatus::Success,
                },
            );
            Ok(())
        }
        ReplyStatus::Failure => {
            record_reply_attempt(
                started_at,
                ReplyAttemptOutcome::Response {
                    status,
                    result: ReplyStatus::Failure,
                },
            );
            Err(VkError::RequestContext {
                context: format!("post reply to {route}").boxed(),
                source: Box::new(std::io::Error::other(format!(
                    "unexpected HTTP status {status}"
                ))),
            })
        }
    }
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

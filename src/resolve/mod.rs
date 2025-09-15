//! Resolve pull request review comments via the GitHub API.
//!
//! This module coordinates REST and GraphQL helpers to optionally post a reply
//! and mark the comment's thread as resolved.

use crate::ref_parser::RepoInfo;
use crate::{VkError, api::GraphQLClient};
use tracing::{Instrument, debug_span};

#[cfg(feature = "unstable-rest-resolve")]
use std::time::Duration;

mod graphql;
#[cfg(feature = "unstable-rest-resolve")]
mod rest;

/// Comment location within a pull request review thread.
#[derive(Copy, Clone, Debug)]
pub struct CommentRef<'a> {
    pub repo: &'a RepoInfo,
    pub pull_number: u64,
    pub comment_id: u64,
}

/// Resolve a pull request review comment and optionally post a reply.
///
/// Returns [`VkError::RequestContext`] if an HTTP request fails.
/// Returns [`VkError::CommentNotFound`] if the comment cannot be located.
///
/// # Examples
///
/// Basic usage:
///
/// ```no_run
/// # use crate::{ref_parser::RepoInfo, resolve::{resolve_comment, CommentRef}, VkError};
/// # async fn run() -> Result<(), VkError> {
/// let repo = RepoInfo { owner: "octocat", name: "hello" };
/// resolve_comment("token", CommentRef { repo: &repo, pull_number: 1, comment_id: 2 }).await?;
/// # Ok(())
/// # }
/// ```
///
/// Resolving and posting a reply (requires `unstable-rest-resolve`):
///
/// ```no_run
/// # use crate::{ref_parser::RepoInfo, resolve::{resolve_comment, CommentRef}, VkError};
/// use std::time::Duration;
/// # async fn run() -> Result<(), VkError> {
/// let repo = RepoInfo { owner: "octocat", name: "hello" };
/// resolve_comment(
///     "token",
///     CommentRef { repo: &repo, pull_number: 1, comment_id: 2 },
///     Some(String::from("Thanks")),
///     Duration::from_secs(10),
///     Duration::from_secs(5),
/// ).await?;
/// # Ok(())
/// # }
/// ```
#[cfg_attr(docsrs, doc(cfg(feature = "unstable-rest-resolve")))]
pub async fn resolve_comment(
    token: &str,
    reference: CommentRef<'_>,
    #[cfg(feature = "unstable-rest-resolve")] message: Option<String>,
    #[cfg(feature = "unstable-rest-resolve")] timeout: Duration,
    #[cfg(feature = "unstable-rest-resolve")] connect_timeout: Duration,
) -> Result<(), VkError> {
    let span = debug_span!(
        "resolve_comment",
        owner = reference.repo.owner,
        repo = reference.repo.name,
        pull_number = reference.pull_number,
        comment_id = reference.comment_id
    );
    let _enter = span.enter();
    #[cfg(feature = "unstable-rest-resolve")]
    if let Some(body) = message.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        let rest_client = rest::RestClient::new(token, None, timeout, connect_timeout)?;
        rest::post_reply(&rest_client, reference, body)
            .instrument(debug_span!("post_reply"))
            .await?;
    }

    let gql = GraphQLClient::new(token, None)?;
    let thread_id = graphql::get_thread_id(&gql, reference)
        .instrument(debug_span!("thread_lookup"))
        .await?;
    graphql::resolve_thread(&gql, &thread_id)
        .instrument(debug_span!("resolve_thread"))
        .await?;
    Ok(())
}

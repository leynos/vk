//! Resolve pull request review comments via the GitHub API.
//!
//! This module coordinates REST and GraphQL helpers to optionally post a reply
//! and mark the comment's thread as resolved.

use crate::ref_parser::RepoInfo;
use crate::{VkError, api::GraphQLClient};

#[cfg(feature = "unstable-rest-resolve")]
use std::time::Duration;

mod graphql;
#[cfg(feature = "unstable-rest-resolve")]
mod rest;

/// Comment location within a pull request review thread.
#[derive(Copy, Clone)]
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
        let rest_client = rest::RestClient::new(token, timeout, connect_timeout)?;
        rest::post_reply(&rest_client, reference, body).await?;
    }

    let gql = GraphQLClient::new(token, None)?;
    let thread_id = graphql::get_thread_id(&gql, reference).await?;
    graphql::resolve_thread(&gql, &thread_id).await?;
    Ok(())
}

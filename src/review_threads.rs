//! Helpers for fetching and filtering pull request review threads from the
//! GitHub API.
//!
//! The module defines GraphQL response structures and helpers to retrieve all
//! unresolved review threads along with their comments. It also provides
//! utilities for filtering threads by file path.

use serde::Deserialize;
use serde_json::{Map, json};
use std::{borrow::Cow, collections::HashSet};

use crate::boxed::BoxedStr;
use crate::graphql_queries::{COMMENT_QUERY, THREADS_QUERY};
use crate::ref_parser::RepoInfo;
use crate::{GraphQLClient, VkError};

#[derive(Debug, Deserialize, Default)]
struct ThreadData {
    repository: Repository,
}

#[derive(Debug, Deserialize, Default)]
struct Repository {
    #[serde(rename = "pullRequest")]
    pull_request: PullRequest,
}

#[derive(Debug, Deserialize, Default)]
struct PullRequest {
    #[serde(rename = "reviewThreads")]
    review_threads: ReviewThreadConnection,
}

#[derive(Debug, Deserialize, Default)]
struct NodeWrapper<T> {
    node: Option<T>,
}

#[derive(Debug, Deserialize, Default)]
struct CommentNode {
    comments: CommentConnection,
}

#[derive(Debug, Deserialize, Default)]
pub struct Connection<T> {
    pub nodes: Vec<T>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

type ReviewThreadConnection = Connection<ReviewThread>;
pub type CommentConnection = Connection<ReviewComment>;

/// Details of a single review thread.
#[derive(Debug, Deserialize, Default)]
pub struct ReviewThread {
    pub id: String,
    #[serde(rename = "isResolved")]
    pub is_resolved: bool,
    pub comments: CommentConnection,
}

/// A single review comment.
#[derive(Debug, Deserialize, Default)]
pub struct ReviewComment {
    pub body: String,
    #[serde(rename = "diffHunk")]
    pub diff_hunk: String,
    #[serde(rename = "originalPosition")]
    pub original_position: Option<i32>,
    pub position: Option<i32>,
    pub path: String,
    pub url: String,
    pub author: Option<User>,
}

/// Pagination information returned by GitHub's GraphQL API.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct PageInfo {
    #[serde(rename = "hasNextPage")]
    pub has_next_page: bool,
    #[serde(rename = "endCursor")]
    pub end_cursor: Option<String>,
}

impl PageInfo {
    /// Return the cursor for the next page when available.
    /// Returns `Ok(None)` when there are no more pages.
    ///
    /// # Errors
    ///
    /// Returns [`VkError::BadResponse`] when `has_next_page` is `true` but
    /// `end_cursor` is absent.
    ///
    /// # Examples
    /// ```
    /// use vk::PageInfo;
    /// let info = PageInfo { has_next_page: true, end_cursor: Some("c1".into()) };
    /// assert_eq!(info.next_cursor().expect("cursor"), Some("c1"));
    /// ```
    /// ```
    /// use vk::PageInfo;
    /// let info = PageInfo { has_next_page: true, end_cursor: None };
    /// assert!(info.next_cursor().is_err());
    /// ```
    /// ```
    /// use vk::PageInfo;
    /// let info = PageInfo { has_next_page: false, end_cursor: None };
    /// assert_eq!(info.next_cursor().expect("cursor"), None);
    /// ```
    #[inline]
    #[must_use = "inspect the returned cursor to advance pagination"]
    pub fn next_cursor(&self) -> Result<Option<&str>, VkError> {
        match (self.has_next_page, self.end_cursor.as_deref()) {
            (true, Some(cursor)) => Ok(Some(cursor)),
            (true, None) => Err(VkError::BadResponse(
                format!(
                    "PageInfo invariant violated: hasNextPage=true but endCursor missing | pageInfo: {self:?}"
                )
                .boxed(),
            )),
            (false, _) => Ok(None),
        }
    }
}

/// Minimal user representation for authorship information.
#[derive(Debug, Deserialize, Default, Clone)]
pub struct User {
    pub login: String,
}

/// Fetch all unresolved review threads for a pull request.
///
/// Note:
/// - GitHub GraphQL `Int` is a 32-bit signed integer (range −2^31..=2^31−1).
///   This function accepts a non-negative `number`; values above `i32::MAX`
///   are rejected with [`VkError::InvalidNumber`].
/// - The token must have sufficient scopes (for example, `repo` for private
///   repositories) or the API may return partial data that fails to
///   deserialise.
///
/// # Errors
///
/// Returns [`VkError::InvalidNumber`] if `number` exceeds `i32::MAX`, or a
/// general [`VkError`] if any API request fails or the response is malformed.
pub async fn fetch_review_threads(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Vec<ReviewThread>, VkError> {
    debug_assert!(
        i32::try_from(number).is_ok(),
        "pull-request number {number} exceeds GraphQL Int (i32) range",
    );
    let number_i32 = i32::try_from(number).map_err(|_| VkError::InvalidNumber)?;

    let mut vars = Map::new();
    vars.insert("owner".into(), json!(repo.owner.clone()));
    vars.insert("name".into(), json!(repo.name.clone()));
    vars.insert("number".into(), json!(number_i32));

    let threads = client
        .paginate_all(THREADS_QUERY, vars, None, |data: ThreadData| {
            let conn = data.repository.pull_request.review_threads;
            Ok((conn.nodes, conn.page_info))
        })
        .await?;

    let mut threads = filter_unresolved_threads(threads);
    for thread in &mut threads {
        let initial = std::mem::take(&mut thread.comments);
        thread.comments = fetch_all_comments(client, &thread.id, initial).await?;
    }
    Ok(threads)
}

/// Fetch all comments for a thread, following pagination when required.
///
/// # Errors
///
/// Propagates any API or pagination errors from the underlying client.
async fn fetch_all_comments(
    client: &GraphQLClient,
    thread_id: &str,
    initial: CommentConnection,
) -> Result<CommentConnection, VkError> {
    let mut comments = initial.nodes;
    if let Some(cursor) = initial.page_info.next_cursor()? {
        let mut vars = Map::new();
        vars.insert("id".into(), json!(thread_id));
        let more = client
            .paginate_all(
                COMMENT_QUERY,
                vars,
                Some(Cow::Borrowed(cursor)),
                move |wrapper: NodeWrapper<CommentNode>| {
                    let conn = wrapper
                        .node
                        .ok_or_else(|| {
                            VkError::BadResponse(
                                format!("Missing comment node in response for thread {thread_id}")
                                    .boxed(),
                            )
                        })?
                        .comments;
                    Ok((conn.nodes, conn.page_info))
                },
            )
            .await?;
        comments.extend(more);
    }
    Ok(CommentConnection {
        nodes: comments,
        page_info: PageInfo {
            has_next_page: false,
            end_cursor: None,
        },
    })
}

/// Retain only unresolved review threads.
///
/// # Examples
///
/// ```ignore
/// use vk::review_threads::{filter_unresolved_threads, ReviewThread};
/// let threads = vec![
///     ReviewThread { is_resolved: true, ..Default::default() },
///     ReviewThread { is_resolved: false, ..Default::default() },
/// ];
/// let filtered = filter_unresolved_threads(threads);
/// assert_eq!(filtered.len(), 1);
/// assert!(!filtered[0].is_resolved);
/// ```
fn filter_unresolved_threads(threads: Vec<ReviewThread>) -> Vec<ReviewThread> {
    threads.into_iter().filter(|t| !t.is_resolved).collect()
}

/// Filter review threads to those whose first comment matches one of `files`.
///
/// Returns the original collection when `files` is empty.
///
/// # Examples
///
/// ```
/// use vk::review_threads::{
///     filter_threads_by_files, CommentConnection, ReviewComment, ReviewThread,
/// };
/// let threads = vec![
///     ReviewThread {
///         comments: CommentConnection {
///             nodes: vec![ReviewComment { path: "src/lib.rs".into(), ..Default::default() }],
///             ..Default::default()
///         },
///         ..Default::default()
///     },
///     ReviewThread {
///         comments: CommentConnection {
///             nodes: vec![ReviewComment { path: "README.md".into(), ..Default::default() }],
///             ..Default::default()
///         },
///         ..Default::default()
///     },
/// ];
/// let filtered = filter_threads_by_files(threads, &[String::from("README.md")]);
/// assert_eq!(filtered.len(), 1);
/// let path = filtered
///     .first()
///     .and_then(|t| t.comments.nodes.first())
///     .map(|c| c.path.as_str());
/// assert_eq!(path, Some("README.md"));
/// ```
pub fn filter_threads_by_files(threads: Vec<ReviewThread>, files: &[String]) -> Vec<ReviewThread> {
    if files.is_empty() {
        return threads;
    }
    let set: HashSet<&str> = files.iter().map(String::as_str).collect();
    threads
        .into_iter()
        .filter(|t| {
            t.comments
                .nodes
                .first()
                .is_some_and(|c| set.contains(c.path.as_str()))
        })
        .collect()
}
#[cfg(test)]
mod tests;

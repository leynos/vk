//! GraphQL wire layer for review-thread fetching.
//!
//! This submodule holds the transport-facing shapes for the review-thread and
//! comment queries: the [`GraphQLQuery`] operations, the [`CursorVariables`]
//! implementations that drive pagination, the private envelope structs used to
//! decode each response, and the public domain types those envelopes populate.
//! Keeping the wire shapes here isolates the generated-type-adjacent decoding
//! from the fetching and filtering behaviour in the parent module.

use graphql_client::GraphQLQuery;
use serde::Deserialize;

use crate::api::CursorVariables;
// `graphql_client` resolves the `URI` scalar (the comment `url` field) to a
// type of the same name in scope of the threads/comment derives; the shared
// alias supplies it.
use crate::VkError;
use crate::api::scalars::URI;
use crate::boxed::BoxedStr;

/// Typed `ThreadsQuery` operation: the paginated review-thread listing.
///
/// The generated `ResponseData` would require `isOutdated` on every thread,
/// but the documented behaviour is that threads missing `isOutdated` are
/// treated as current (see `docs/vk-design.md`). The response is therefore
/// decoded into the hand-written [`ThreadData`] via
/// [`GraphQLClient::paginate_operation_as`], which keeps that `#[serde(default)]`
/// leniency while still validating the query against the vendored schema.
///
/// [`GraphQLClient::paginate_operation_as`]: crate::GraphQLClient::paginate_operation_as
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/schema.docs.graphql",
    query_path = "graphql/review_threads.graphql",
    variables_derives = "Clone",
    response_derives = "Debug, Clone, PartialEq"
)]
pub struct ThreadsQuery;

/// Typed `CommentQuery` operation: per-thread comment paging via `node(id:)`.
///
/// Decoded into the hand-written [`NodeWrapper<CommentNode>`] for consistency
/// with [`ThreadsQuery`] (both populate the shared [`ReviewComment`]) and to
/// avoid mapping the `Node` interface's inline-fragment enum; the fixtures
/// would also satisfy the generated `ResponseData`.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/schema.docs.graphql",
    query_path = "graphql/review_threads.graphql",
    variables_derives = "Clone",
    response_derives = "Debug, Clone, PartialEq"
)]
pub struct CommentQuery;

impl CursorVariables for threads_query::Variables {
    fn set_cursor(&mut self, cursor: Option<String>) {
        self.cursor = cursor;
    }
}

impl CursorVariables for comment_query::Variables {
    fn set_cursor(&mut self, cursor: Option<String>) {
        self.cursor = cursor;
    }
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct ThreadData {
    pub(super) repository: Repository,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct Repository {
    #[serde(rename = "pullRequest")]
    pub(super) pull_request: PullRequest,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct PullRequest {
    #[serde(rename = "reviewThreads")]
    pub(super) review_threads: ReviewThreadConnection,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct NodeWrapper<T> {
    pub(super) node: Option<T>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct CommentNode {
    pub(super) comments: CommentConnection,
}

#[derive(Debug, Deserialize, Default)]
pub struct Connection<T> {
    pub nodes: Vec<T>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

pub(super) type ReviewThreadConnection = Connection<ReviewThread>;
pub type CommentConnection = Connection<ReviewComment>;

/// Details of a single review thread.
#[derive(Debug, Deserialize, Default)]
pub struct ReviewThread {
    pub id: String,
    #[serde(rename = "isResolved")]
    pub is_resolved: bool,
    #[serde(default, rename = "isOutdated")]
    pub is_outdated: bool,
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

//! GraphQL wire layer for pull-request review fetching.
//!
//! This submodule holds the transport-facing shapes for the reviews query: the
//! [`GraphQLQuery`] operation, the [`CursorVariables`] implementation that
//! drives pagination, the private envelope structs used to decode the
//! response, and the public [`PullRequestReview`] domain type those envelopes
//! populate. Keeping the wire shapes here isolates the generated-type-adjacent
//! decoding from the collating behaviour in the parent module.

use graphql_client::GraphQLQuery;
use serde::Deserialize;

// `graphql_client` resolves the `DateTime` scalar in `reviews.graphql` to a
// type of the same name in scope of the derive; the shared alias supplies it.
// It is `chrono::DateTime<chrono::Utc>`, so it doubles as the field type for
// `PullRequestReview::submitted_at`.
use crate::api::CursorVariables;
use crate::api::scalars::DateTime;
use crate::{PageInfo, User};

/// Typed `ReviewsQuery` operation: the paginated pull-request review listing.
///
/// The response is decoded into the hand-written [`ReviewData`] via
/// [`GraphQLClient::paginate_operation_as`] rather than the generated
/// `ResponseData` because the schema types `state` as the enum
/// `PullRequestReviewState`, whereas the public [`PullRequestReview::state`] is
/// a `String` that must preserve the wire value verbatim (including any future
/// or unknown state). Decoding into the hand-written struct avoids an
/// enum round-trip while still validating the query at compile time.
///
/// [`GraphQLClient::paginate_operation_as`]: crate::GraphQLClient::paginate_operation_as
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/schema.docs.graphql",
    query_path = "graphql/reviews.graphql",
    variables_derives = "Clone",
    response_derives = "Debug, Clone, PartialEq"
)]
pub struct ReviewsQuery;

impl CursorVariables for reviews_query::Variables {
    fn set_cursor(&mut self, cursor: Option<String>) {
        self.cursor = cursor;
    }
}

/// Public domain representation of a pull-request review returned by GitHub.
///
/// Its [`state`](Self::state) preserves GitHub's wire value verbatim because
/// decoding bypasses the generated `PullRequestReviewState` enum.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestReview {
    /// Review body in Markdown.
    pub body: String,
    /// Timestamp when the review was formally submitted.
    ///
    /// This may be `None` when the timestamp is missing or unknown.
    pub submitted_at: Option<DateTime>,
    /// Review state as returned by GitHub, preserved verbatim because the
    /// generated enum is bypassed during decoding.
    pub state: String,
    /// Review author, when GitHub provides one.
    pub author: Option<User>,
}

/// Envelope for one reviews query response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewData {
    /// Repository that owns the requested pull request.
    pub(super) repository: RepositoryReviews,
}

/// Repository envelope in a reviews response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RepositoryReviews {
    /// Pull request whose reviews were requested.
    #[serde(rename = "pullRequest")]
    pub(super) pull_request: PullRequestReviews,
}

/// Pull-request envelope in a reviews response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PullRequestReviews {
    /// Paginated reviews from the pull request.
    pub(super) reviews: ReviewConnection,
}

/// Paginated reviews decoded into public domain values.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewConnection {
    /// Non-null reviews from this connection page.
    #[serde(default, deserialize_with = "crate::api::deserialize::nullable_nodes")]
    pub(super) nodes: Vec<PullRequestReview>,
    /// Pagination metadata for this reviews page.
    pub(super) page_info: PageInfo,
}

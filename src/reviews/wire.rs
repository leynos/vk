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

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestReview {
    pub body: String,
    /// Timestamp when the review was formally submitted.
    ///
    /// This may be `None` when the timestamp is missing or unknown.
    pub submitted_at: Option<DateTime>,
    pub state: String,
    pub author: Option<User>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewData {
    pub(super) repository: RepositoryReviews,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RepositoryReviews {
    #[serde(rename = "pullRequest")]
    pub(super) pull_request: PullRequestReviews,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PullRequestReviews {
    pub(super) reviews: ReviewConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewConnection {
    pub(super) nodes: Vec<PullRequestReview>,
    pub(super) page_info: PageInfo,
}

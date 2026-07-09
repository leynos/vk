//! Helpers for fetching issues from the GitHub API.
//!
//! Currently only retrieval of a single issue by number is supported.
//!
//! This module also sets the crate convention for `graphql_client` codegen:
//! each consumer module hosts its own `#[derive(GraphQLQuery)]` item (pointing
//! at the vendored schema and an operation document under `graphql/`) together
//! with the mapper that converts the generated `ResponseData` into the public
//! domain type. Custom scalars, when an operation needs them, come from
//! `crate::api::scalars`.

use crate::boxed::BoxedStr;
use crate::ref_parser::RepoInfo;
use crate::{GraphQLClient, VkError};
use graphql_client::GraphQLQuery;
use serde::Deserialize;

/// Typed `IssueQuery` GraphQL operation.
///
/// Generated from `graphql/issue.graphql` and validated against the vendored
/// schema at compile time. The generated module `issue_query` provides the
/// `Variables` and `ResponseData` types used by [`fetch_issue`].
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/schema.docs.graphql",
    query_path = "graphql/issue.graphql",
    response_derives = "Debug, Clone, PartialEq"
)]
pub struct IssueQuery;

/// Minimal issue representation returned by the GitHub API.
///
/// `Deserialize` is retained for consumers that decode fixtures directly; the
/// GraphQL path populates this type through [`issue_from_response`].
#[derive(Deserialize)]
pub struct Issue {
    pub title: String,
    pub body: String,
}

/// Convert an `IssueQuery` response into the public [`Issue`] type.
///
/// A missing `repository` or `issue` node (both nullable in the schema) means
/// the requested issue does not exist; this is surfaced as a [`VkError`] rather
/// than a panic, matching the pre-codegen behaviour where the required-field
/// deserialization failed on a null issue.
fn issue_from_response(data: issue_query::ResponseData, number: u64) -> Result<Issue, VkError> {
    let issue = data
        .repository
        .and_then(|repository| repository.issue)
        .ok_or_else(|| VkError::BadResponse(format!("issue #{number} not found").boxed()))?;
    Ok(Issue {
        title: issue.title,
        body: issue.body,
    })
}

/// Fetch a single issue by repository and number.
///
/// # Errors
///
/// Returns an error if the issue number is out of range, the API request
/// fails, or the response is malformed or reports no such issue.
pub async fn fetch_issue(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Issue, VkError> {
    let variables = issue_query::Variables {
        owner: repo.owner.clone(),
        name: repo.name.clone(),
        // GraphQL `Int` maps to `i64`; issue numbers beyond `i64::MAX` cannot
        // exist, so an overflow indicates a malformed reference.
        number: i64::try_from(number).map_err(|_| VkError::InvalidNumber)?,
    };
    let data = client.run_operation::<IssueQuery>(variables).await?;
    issue_from_response(data, number)
}

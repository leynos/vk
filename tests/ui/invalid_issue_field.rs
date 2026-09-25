//! `trybuild` compile-fail fixture for an invalid `graphql_client` operation.
//!
//! It deliberately selects a nonexistent field to verify schema validation.

use graphql_client::GraphQLQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "../../../../graphql/schema.docs.graphql",
    query_path = "../../../../tests/ui/invalid_issue_field.graphql"
)]
struct InvalidIssueField;

fn main() {}

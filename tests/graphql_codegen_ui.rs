//! UI tests for compile-time GraphQL document validation.

#[test]
fn rejects_unknown_graphql_fields() {
    trybuild::TestCases::new().compile_fail("tests/ui/invalid_issue_field.rs");
}

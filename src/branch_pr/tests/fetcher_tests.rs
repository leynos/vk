//! Tests for the branch lookup's injected fetcher boundary.
//!
//! These pin the adapter contract directly, without an HTTP stub: the
//! traversal must issue exactly the cursor sequence the server dictates, and
//! the real `GraphQLClient` must remain a valid stand-in for the mock.

use super::*;
use crate::branch_pr::wire::{PrForBranchFetcher, PrPageRequest};

/// The injected fetcher must see the cursor sequence the traversal issues.
#[tokio::test]
async fn traversal_advances_the_cursor_through_the_injected_fetcher() {
    let repo = RepoInfo {
        owner: "owner".into(),
        name: "repo".into(),
    };
    let mut mock = MockPrForBranchFetcher::new();
    mock.expect_fetch_pr_page()
        .times(1)
        .withf(|request: &PrPageRequest<'_>| request.after.is_none())
        .returning(|_| Ok(page(vec![], true, Some("page-2"))));
    mock.expect_fetch_pr_page()
        .times(1)
        .withf(|request: &PrPageRequest<'_>| request.after.as_deref() == Some("page-2"))
        .returning(|_| {
            Ok(page(
                vec![CandidatePr {
                    number: 42,
                    head_owner: Some("target-owner".into()),
                }],
                false,
                None,
            ))
        });

    let number = find_pr_number(&mock, &repo, "feature", Some("target-owner"))
        .await
        .expect("match on second page");

    assert_eq!(number, 42);
}

/// The concrete adapter must satisfy the trait the traversal is generic over,
/// so the mock above stands in for the real client faithfully.
#[test]
fn graphql_client_implements_the_page_fetcher() {
    fn assert_impl<T: PrForBranchFetcher>() {}
    assert_impl::<GraphQLClient>();
}

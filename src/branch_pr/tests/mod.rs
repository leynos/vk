//! Tests for `branch_pr` module.
//!
//! Wire-envelope decoding is covered by the submodule tests in [`super::wire`];
//! these tests exercise the owner-matching policy and the mocked pagination
//! traversal that `find_pr_number` owns.

use super::*;
use mockall::Sequence;
use rstest::rstest;

/// Build a candidate with an optional head owner.
fn candidate(number: u64, head_owner: Option<&str>) -> CandidatePr {
    CandidatePr {
        number,
        head_owner: head_owner.map(ToOwned::to_owned),
    }
}

/// Build a page around the given candidates.
fn page(prs: Vec<CandidatePr>, has_next_page: bool, end_cursor: Option<&str>) -> PrPage {
    PrPage {
        prs,
        page_info: PageInfo {
            has_next_page,
            end_cursor: end_cursor.map(ToOwned::to_owned),
        },
    }
}

#[test]
fn filter_prs_by_head_owner() {
    let prs = [
        candidate(1, Some("other-owner")),
        candidate(2, Some("target-owner")),
        candidate(3, None), // Deleted fork
    ];

    // Find PR by head owner using the helper
    let matching = prs.iter().find(|pr| pr.head_owner_matches("target-owner"));
    assert_eq!(matching.expect("found PR").number, 2);

    // Case-insensitive match
    let matching_case = prs.iter().find(|pr| pr.head_owner_matches("TARGET-OWNER"));
    assert_eq!(matching_case.expect("found PR").number, 2);

    // No match for unknown owner
    let no_match = prs.iter().find(|pr| pr.head_owner_matches("unknown"));
    assert!(no_match.is_none());
}

#[rstest]
#[case::matching(Some("fork-owner"), "fork-owner", true)]
#[case::case_insensitive(Some("Fork-Owner"), "fork-owner", true)]
#[case::different_owner(Some("other-owner"), "fork-owner", false)]
#[case::deleted_fork(None, "fork-owner", false)]
fn head_owner_matches_reports_candidate_eligibility(
    #[case] head_owner: Option<&str>,
    #[case] filter: &str,
    #[case] expected: bool,
) {
    assert_eq!(
        candidate(1, head_owner).head_owner_matches(filter),
        expected
    );
}

#[tokio::test]
async fn stops_at_the_first_page_without_a_next_cursor() {
    let mut mock = MockPrForBranchFetcher::new();
    mock.expect_fetch_pr_page()
        .times(1)
        .returning(|_| Ok(page(vec![], false, None)));
    let repo = RepoInfo {
        owner: "o".into(),
        name: "r".into(),
    };

    let error = find_pr_number(&mock, &repo, "feature", None)
        .await
        .expect_err("no candidate to match");

    assert!(matches!(error, VkError::NoPrForBranch { branch } if branch.as_ref() == "feature"));
}

#[tokio::test]
async fn requests_the_cursor_returned_by_the_previous_page() {
    let mut mock = MockPrForBranchFetcher::new();
    let mut seq = Sequence::new();
    mock.expect_fetch_pr_page()
        .times(1)
        .in_sequence(&mut seq)
        .withf(|request| request.after.is_none() && request.branch == "feature")
        .returning(|_| {
            Ok(page(
                vec![candidate(1, Some("other-owner"))],
                true,
                Some("page-2"),
            ))
        });
    mock.expect_fetch_pr_page()
        .times(1)
        .in_sequence(&mut seq)
        .withf(|request| request.after.as_deref() == Some("page-2"))
        .returning(|_| Ok(page(vec![candidate(42, Some("target-owner"))], false, None)));
    let repo = RepoInfo {
        owner: "o".into(),
        name: "r".into(),
    };

    let number = find_pr_number(&mock, &repo, "feature", Some("target-owner"))
        .await
        .expect("second page match");

    assert_eq!(number, 42);
}

mod fetch_pr_for_branch_tests;

mod fetcher_tests;

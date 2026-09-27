//! Utilities for resolving pull requests from the current Git branch.
//!
//! This module provides functions to look up pull requests associated with a
//! branch via the GitHub GraphQL API. Supports disambiguation when multiple
//! forks have PRs with the same branch name by filtering on the head repository
//! owner.
//!
//! The GraphQL operation itself lives in the private [`wire`] submodule, which
//! owns the `graphql_client` codegen item, the response envelopes, and the
//! [`GraphQLClient`] call. This module owns the pagination traversal and the
//! matching policy, and speaks only the domain shapes defined here.

use crate::api::{CursorHistory, page_limit_error, page_limit_exceeded};
use crate::ref_parser::RepoInfo;
use crate::{GraphQLClient, PageInfo, VkError};

mod wire;

#[cfg(test)]
pub(crate) use wire::MockPrForBranchFetcher;
use wire::{PrForBranchFetcher, PrPageRequest};

/// One candidate pull request from a page of branch lookup results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidatePr {
    /// Pull-request number.
    pub(crate) number: u64,
    /// Login of the head repository owner, when GitHub reports one.
    ///
    /// This is `None` for a pull request whose head repository has been
    /// deleted, which no owner filter can match.
    pub(crate) head_owner: Option<String>,
}

impl CandidatePr {
    /// Check if this candidate's head repository owner matches `owner`,
    /// ignoring ASCII case.
    pub(crate) fn head_owner_matches(&self, owner: &str) -> bool {
        self.head_owner
            .as_deref()
            .is_some_and(|login| login.eq_ignore_ascii_case(owner))
    }
}

/// One page of branch pull-request candidates and its pagination metadata.
#[derive(Debug, Clone)]
pub(crate) struct PrPage {
    /// Candidates on this page, in the order GitHub returned them.
    pub(crate) prs: Vec<CandidatePr>,
    /// Pagination metadata for this page.
    pub(crate) page_info: PageInfo,
}

/// Look up the pull request number for a branch via the GitHub API.
///
/// Queries for open or merged PRs with the given branch as the head ref. When
/// `head_owner` is provided, filters results to match the head repository owner,
/// disambiguating between multiple forks with the same branch name.
///
/// # Arguments
///
/// * `client` - The GraphQL client for API requests
/// * `repo` - The target repository (where the PR is opened against)
/// * `branch` - The head branch name to search for
/// * `head_owner` - Optional owner of the head repository (the fork owner). When
///   `None`, returns the first matching PR without filtering.
///
/// # Errors
///
/// Returns [`VkError::NoPrForBranch`] if no PR exists for the branch (or no PR
/// matches the specified head owner), or propagates API errors from the
/// underlying request.
///
/// # Example
///
/// ```ignore
/// use crate::{GraphQLClient, VkError};
/// use crate::ref_parser::RepoInfo;
/// use crate::branch_pr::fetch_pr_for_branch;
///
/// async fn demo(client: &GraphQLClient) -> Result<(), VkError> {
///     let repo = RepoInfo {
///         owner: "upstream".into(),
///         name: "project".into(),
///     };
///     let pr_number = fetch_pr_for_branch(
///         client,
///         &repo,
///         "feature-branch",
///         Some("fork-owner"),
///     ).await?;
///     println!("Found PR #{pr_number}");
///     Ok(())
/// }
/// ```
pub async fn fetch_pr_for_branch(
    client: &GraphQLClient,
    repo: &RepoInfo,
    branch: &str,
    head_owner: Option<&str>,
) -> Result<u64, VkError> {
    find_pr_number(client, repo, branch, head_owner).await
}

/// Page through branch pull-request candidates until one matches.
///
/// The traversal is bounded by [`page_limit_exceeded`] and refuses a cursor
/// that would repeat, so an uncooperative server cannot make it loop forever.
pub(crate) async fn find_pr_number(
    fetcher: &impl PrForBranchFetcher,
    repo: &RepoInfo,
    branch: &str,
    head_owner: Option<&str>,
) -> Result<u64, VkError> {
    let mut after = None;
    let mut cursor_history = CursorHistory::new(after.as_deref());
    let mut pages_seen = 0usize;
    loop {
        pages_seen += 1;
        if page_limit_exceeded(pages_seen) {
            return Err(page_limit_error());
        }
        let request_cursor = after.take();
        let page = fetcher
            .fetch_pr_page(PrPageRequest {
                owner: &repo.owner,
                name: &repo.name,
                branch,
                after: request_cursor.clone(),
            })
            .await?;
        let matching_pr = head_owner.map_or_else(
            || page.prs.first(),
            |owner| page.prs.iter().find(|pr| pr.head_owner_matches(owner)),
        );
        if let Some(pr) = matching_pr {
            return Ok(pr.number);
        }
        let Some(cursor) = page.page_info.next_cursor()? else {
            break;
        };
        cursor_history.record_next(cursor)?;
        after = Some(cursor.to_owned());
    }
    Err(VkError::NoPrForBranch {
        branch: branch.into(),
    })
}

#[cfg(test)]
mod tests;

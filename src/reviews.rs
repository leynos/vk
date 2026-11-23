//! Functions for retrieving pull-request reviews through the GitHub API.
//!
//! The module defines GraphQL query structures and pagination helpers so callers
//! can fetch pull-request reviews and collate the latest review from each author.

use crate::{GraphQLClient, VkError, ref_parser::RepoInfo};
use std::collections::{HashMap, hash_map::Entry};

mod wire;

pub use wire::{PullRequestReview, ReviewsQuery};
use wire::{ReviewData, reviews_query};

/// Retrieve all reviews for a pull request by paging through the GitHub
/// GraphQL API.
///
/// Note:
/// - GitHub GraphQL `Int` is a 32-bit signed integer (range −2^31..=2^31−1).
///   This function accepts a non-negative `number`; values above `i32::MAX`
///   are rejected with [`VkError::InvalidNumber`].
/// - The token must have sufficient scopes (for example, `repo` for private
///   repositories) or the API may return partial data that fails to
///   deserialise.
///
/// ```no_run
/// use vk::{GraphQLClient, ref_parser::RepoInfo};
///
/// # async fn run() -> Result<(), vk::VkError> {
/// let client = GraphQLClient::new("token", None).expect("client");
/// let repo = RepoInfo { owner: "octocat".into(), name: "hello-world".into() };
/// let reviews = vk::reviews::fetch_reviews(&client, &repo, 1).await?;
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns [`VkError::InvalidNumber`] if `number` exceeds `i32::MAX`, or a
/// [`VkError`] if the request fails or the response cannot be deserialised.
pub async fn fetch_reviews(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Vec<PullRequestReview>, VkError> {
    // GraphQL `Int` maps to `i64`; a pull-request number beyond `i32::MAX`
    // cannot be valid, so range-check as `i32` before widening.
    let number = i64::from(i32::try_from(number).map_err(|_| VkError::InvalidNumber)?);

    let variables = reviews_query::Variables {
        owner: repo.owner.clone(),
        name: repo.name.clone(),
        number,
        cursor: None,
    };
    client
        .paginate_operation_as::<ReviewsQuery, ReviewData, PullRequestReview, _>(
            variables,
            None,
            |data: ReviewData| {
                let conn = data.repository.pull_request.reviews;
                Ok((conn.nodes, conn.page_info))
            },
        )
        .await
}

/// Determine whether `new` should replace `existing` when collating reviews.
///
/// Prefer reviews with a timestamp over those without. When both have
/// timestamps, keep the later one. Tie-break on equal timestamps (or both
/// `None`) by favouring the later item in input order.
///
/// The input order is the order GitHub returned the reviews in, across every
/// page. GitHub documents no ordering guarantee for the `reviews` connection,
/// so this tie-break is only as good as that order; it never overrides a
/// timestamp comparison, which is the part that carries meaning.
#[expect(
    clippy::match_same_arms,
    reason = "arms kept separate for readability of tie-breaking rules"
)]
fn is_dominated(new: &PullRequestReview, existing: &PullRequestReview) -> bool {
    match (new.submitted_at, existing.submitted_at) {
        (Some(new_ts), Some(old_ts)) => new_ts >= old_ts,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => true,
    }
}

/// Select the most recent review from each author.
///
/// Reviews without an author are returned individually rather than being
/// grouped together.
///
/// Ordering:
/// - The order of reviews with authors is not guaranteed.
/// - Anonymous reviews are appended after the keyed results.
///
/// Tie-break:
/// - When two reviews from the same author share the same `submitted_at`,
///   the later item in the input sequence wins. For reviews read through
///   [`fetch_reviews`] that sequence is GitHub's page order, which the API
///   does not promise to be chronological; the rule only decides cases where
///   the timestamps are equal anyway.
///
/// If you require a deterministic order, sort the returned vector by
/// `submitted_at` at the call site.
///
/// ```
/// use chrono::Utc;
/// use vk::reviews::{latest_reviews, PullRequestReview};
///
/// let reviews = vec![
///     PullRequestReview {
///         body: String::new(),
///         submitted_at: Some(Utc::now()),
///         state: "COMMENTED".into(),
///         author: None,
///     },
///     PullRequestReview {
///         body: String::new(),
///         submitted_at: Some(Utc::now()),
///         state: "COMMENTED".into(),
///         author: None,
///     },
/// ];
/// let latest = latest_reviews(reviews);
/// assert_eq!(latest.len(), 2);
/// ```
pub fn latest_reviews(reviews: Vec<PullRequestReview>) -> Vec<PullRequestReview> {
    let mut latest: HashMap<String, PullRequestReview> = HashMap::new();
    let mut anonymous = Vec::new();
    for r in reviews {
        // GitHub canonicalises login casing, so case collisions are not
        // expected when keying by `login`.
        match r.author.as_ref().map(|u| u.login.clone()) {
            Some(login) => match latest.entry(login) {
                Entry::Vacant(e) => {
                    e.insert(r);
                }
                Entry::Occupied(mut e) => {
                    if is_dominated(&r, e.get()) {
                        e.insert(r);
                    }
                }
            },
            None => anonymous.push(r),
        }
    }
    let keyed = latest.into_values();
    let mut out = Vec::with_capacity(keyed.len() + anonymous.len());
    out.extend(keyed);
    out.extend(anonymous);
    out
}

#[cfg(test)]
mod tests;

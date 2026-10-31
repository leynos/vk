//! Branch and repository auto-detection for the pull-request subcommand.
//!
//! Resolves the current branch, the target repository and the head
//! repository owner from the command line, `FETCH_HEAD` and the `origin`
//! remote, in that order.

use crate::VkError;
use crate::ref_parser::{
    RepoInfo, current_branch, parse_repo_str, repo_from_fetch_head, repo_from_origin,
};
use tracing::debug;

/// Context for branch-based PR auto-detection.
pub(super) struct BranchContext {
    /// The target repository (where the PR is opened against).
    pub(super) repo: RepoInfo,
    /// The current branch name.
    pub(super) branch: String,
    /// The head repository owner (from origin remote), used to disambiguate
    /// when multiple forks have PRs with the same branch name.
    pub(super) head_owner: Option<String>,
}

/// Resolve the current branch and repository for PR auto-detection.
///
/// Returns the target repository, the current branch name, and optionally the
/// head repository owner (from the `origin` remote URL) for disambiguating PRs
/// from forks.
///
/// The target repository is resolved from, in order: the `--repo` flag,
/// `FETCH_HEAD`, then the `origin` remote URL. The `origin` fallback handles
/// fresh worktrees where `git fetch` has not been run since creation, so
/// `FETCH_HEAD` does not yet exist. `FETCH_HEAD` is consulted before `origin`
/// because in fork workflows it points at the upstream repository (where PRs
/// live) while `origin` points at the user's fork.
///
/// `repo_from_origin` is called lazily so the happy path (a usable `--repo` or
/// `FETCH_HEAD`) only spawns git once — for the head-owner lookup. The rare
/// `origin` fallback pays for a second invocation to keep the fast path
/// allocation-free.
///
/// # Errors
///
/// Returns `VkError::DetachedHead` when the repository is in detached HEAD
/// state, or `VkError::RepoNotFound` when the repository cannot be determined.
pub(super) fn resolve_branch_and_repo(
    default_repo: Option<&str>,
) -> Result<BranchContext, VkError> {
    let branch = current_branch().ok_or(VkError::DetachedHead)?;
    // Each `.inspect(...)` fires only when its `Option` is `Some`, so exactly
    // one `debug!` runs and it identifies the winning source. The chain stops
    // short-circuiting at the first match, so subsequent sources are not
    // consulted (and not logged).
    let repo = default_repo
        .and_then(parse_repo_str)
        .inspect(|r| debug!(repo = %format_repo(r), "resolved repo from --repo"))
        .or_else(|| {
            repo_from_fetch_head()
                .inspect(|r| debug!(repo = %format_repo(r), "resolved repo from FETCH_HEAD"))
        })
        .or_else(|| {
            repo_from_origin()
                .inspect(|r| debug!(repo = %format_repo(r), "resolved repo from origin remote"))
        })
        .ok_or(VkError::RepoNotFound)?;
    let head_owner = repo_from_origin().map(|r| r.owner);
    Ok(BranchContext {
        repo,
        branch,
        head_owner,
    })
}

/// Format a repository as its `owner/name` path.
pub(super) fn format_repo(repo: &RepoInfo) -> String {
    format!("{}/{}", repo.owner, repo.name)
}

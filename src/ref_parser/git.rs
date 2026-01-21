//! Git repository detection helpers.
//!
//! Provides functions for detecting repository information from git state,
//! including current branch, origin remote URL, and `FETCH_HEAD`.

use std::path::Path;
use std::{fs, process::Command};

use super::{RepoInfo, parse_repo_str};

/// Internal implementation of branch detection that accepts an optional directory.
///
/// When `dir` is `Some`, runs git in that directory; otherwise uses the current
/// working directory.
pub(crate) fn current_branch_impl(dir: Option<&Path>) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(["symbolic-ref", "--short", "HEAD"]);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        // Fails on detached HEAD or outside a git repo
        return None;
    }
    let branch = String::from_utf8(output.stdout).ok()?;
    Some(branch.trim().to_string())
}

/// Extract the current branch name using `git symbolic-ref`.
///
/// Uses `git symbolic-ref --short HEAD` to resolve the branch name, which
/// works correctly with worktrees, linked gitdirs, and unborn branches where
/// `.git` may be a file rather than a directory.
///
/// Returns `None` if not inside a Git repository, in detached HEAD state,
/// or if the command fails.
///
/// # Examples
///
/// ```ignore
/// // When on branch "feature-branch"
/// assert_eq!(current_branch(), Some("feature-branch".to_string()));
/// ```
pub fn current_branch() -> Option<String> {
    current_branch_impl(None)
}

/// Internal implementation of `FETCH_HEAD` parsing that accepts an optional directory.
///
/// When `dir` is `Some`, runs git in that directory and resolves paths relative
/// to it; otherwise uses the current working directory.
pub(crate) fn repo_from_fetch_head_impl(dir: Option<&Path>) -> Option<RepoInfo> {
    let mut cmd = Command::new("git");
    cmd.args(["rev-parse", "--git-path", "FETCH_HEAD"]);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let rel_path = String::from_utf8(output.stdout).ok()?;
    // git rev-parse --git-path returns a path relative to the working directory
    let full_path = dir.map_or_else(
        || std::path::PathBuf::from(rel_path.trim()),
        |d| d.join(rel_path.trim()),
    );
    let content = fs::read_to_string(full_path).ok()?;
    content.lines().find_map(parse_repo_str)
}

/// Extract repository information from `FETCH_HEAD`.
///
/// Uses `git rev-parse --git-path FETCH_HEAD` to resolve the actual path to
/// the `FETCH_HEAD` file, which works correctly with worktrees and linked
/// gitdirs where `.git` may be a file rather than a directory.
///
/// Parses the first matching GitHub URL from the `FETCH_HEAD` file, which is
/// written after `git fetch` operations.
pub fn repo_from_fetch_head() -> Option<RepoInfo> {
    repo_from_fetch_head_impl(None)
}

/// Internal implementation of origin remote URL parsing that accepts an optional directory.
///
/// When `dir` is `Some`, runs git in that directory; otherwise uses the current
/// working directory.
pub(crate) fn repo_from_origin_impl(dir: Option<&Path>) -> Option<RepoInfo> {
    let mut cmd = Command::new("git");
    cmd.args(["remote", "get-url", "origin"]);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8(output.stdout).ok()?;
    parse_repo_str(url.trim())
}

/// Extract repository information from the `origin` remote URL.
///
/// Uses `git remote get-url origin` to retrieve the URL, which works correctly
/// with worktrees and linked gitdirs. This identifies the user's fork when
/// working on a forked repository.
///
/// Returns `None` if the `origin` remote is not configured or the URL cannot
/// be parsed as a GitHub repository.
///
/// # Examples
///
/// ```ignore
/// // When origin points to a GitHub repository
/// let repo = repo_from_origin().expect("origin configured");
/// assert_eq!(repo.owner, "fork-owner");
/// assert_eq!(repo.name, "repo");
/// ```
pub fn repo_from_origin() -> Option<RepoInfo> {
    repo_from_origin_impl(None)
}

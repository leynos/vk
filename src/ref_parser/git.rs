//! Git repository detection helpers.
//!
//! Provides functions for detecting repository information from git state,
//! including current branch, origin remote URL, and `FETCH_HEAD`.

use std::path::Path;
use std::process::Command;

use super::{RepoInfo, parse_repo_str};

/// Extract the current branch name using `git symbolic-ref --short HEAD`.
///
/// This works with worktrees, linked gitdirs, and unborn branches where `.git`
/// may be a file rather than a directory. Returns `None` outside a Git
/// repository, in detached HEAD state, or if the command fails.
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

/// Extract repository information from `FETCH_HEAD`.
///
/// Uses `git rev-parse --git-path FETCH_HEAD` to find the file, which works
/// with worktrees and linked gitdirs, then parses the first GitHub URL that
/// `git fetch` wrote.
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
    let content = crate::fs_access::read_to_string(&full_path).ok()?;
    content.lines().find_map(parse_repo_str)
}

/// Extract repository information from the `origin` remote URL.
///
/// Uses `git remote get-url origin`, which identifies the user's fork when
/// working on a fork. Returns `None` if `origin` is not configured or is not a
/// GitHub URL.
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

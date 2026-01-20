//! Parse pull request and issue references into repository and number pairs, optionally including discussion comment IDs.

use std::path::Path;
use std::{fs, process::Command, sync::LazyLock};

use regex::Regex;
use url::Url;

use crate::VkError;

#[cfg(test)]
mod tests;

/// Fragment prefix for discussion comment IDs in GitHub URLs.
const DISCUSSION_FRAGMENT: &str = "#discussion_r";

#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub owner: String,
    pub name: String,
}

/// Optional default repository for resolving bare numeric references.
///
/// When parsing a bare number (e.g., "42"), the default repository provides
/// the owner/repo context. If not provided, falls back to `FETCH_HEAD`.
#[derive(Debug, Clone, Copy)]
pub struct DefaultRepo<'a>(Option<&'a str>);

impl<'a> DefaultRepo<'a> {
    /// Create a default repository reference from an owner/repo string.
    #[allow(dead_code, reason = "public API for callers to construct DefaultRepo")]
    pub fn new(repo: &'a str) -> Self {
        Self(Some(repo))
    }

    /// No default repository; will fall back to `FETCH_HEAD` for numeric refs.
    #[allow(dead_code, reason = "public API for callers to construct DefaultRepo")]
    pub const fn none() -> Self {
        Self(None)
    }

    /// Get the inner `Option<&str>`.
    pub(crate) fn as_option(self) -> Option<&'a str> {
        self.0
    }
}

impl<'a> From<Option<&'a str>> for DefaultRepo<'a> {
    fn from(opt: Option<&'a str>) -> Self {
        Self(opt)
    }
}

impl<'a> From<&'a str> for DefaultRepo<'a> {
    fn from(s: &'a str) -> Self {
        Self(Some(s))
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ResourceType {
    Issues,
    PullRequest,
}

impl ResourceType {
    fn allowed_segments(self) -> &'static [&'static str] {
        match self {
            Self::Issues => &["issues", "issue"],
            Self::PullRequest => &["pull", "pulls"],
        }
    }
}

static GITHUB_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"github\.com[/:](?P<owner>[^/]+)/(?P<repo>[^/]+)").expect("valid regex")
});

fn strip_git_suffix(name: &str) -> &str {
    name.strip_suffix(".git").unwrap_or(name)
}

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

fn parse_github_url(
    input: &str,
    resource: ResourceType,
) -> Option<Result<(RepoInfo, u64), VkError>> {
    let url = Url::parse(input).ok()?;
    if url.host_str()? != "github.com" {
        return None;
    }
    let parts: Vec<_> = url.path_segments()?.collect();
    match parts.as_slice() {
        [owner, repo_part, segment, number_str, ..] => {
            if !resource.allowed_segments().contains(segment) {
                return Some(Err(VkError::WrongResourceType {
                    expected: resource.allowed_segments(),
                    found: (*segment).into(),
                }));
            }
            let Ok(number) = number_str.parse() else {
                return Some(Err(VkError::InvalidRef));
            };
            let repo = RepoInfo {
                owner: (*owner).into(),
                name: strip_git_suffix(repo_part).into(),
            };
            Some(Ok((repo, number)))
        }
        _ => Some(Err(VkError::InvalidRef)),
    }
}

/// Parse a repository string into owner and name components.
///
/// Accepts GitHub URLs (`github.com[/:]owner/repo[.git]`) or short format
/// (`owner/repo`).
///
/// # Examples
///
/// ```
/// # use vk::ref_parser::parse_repo_str;
/// let repo = parse_repo_str("owner/repo").expect("valid owner/repo string");
/// assert_eq!(repo.owner, "owner");
/// assert_eq!(repo.name, "repo");
/// ```
pub fn parse_repo_str(repo: &str) -> Option<RepoInfo> {
    if let Some(caps) = GITHUB_RE.captures(repo) {
        let owner = caps.name("owner")?.as_str().to_owned();
        let name = strip_git_suffix(caps.name("repo")?.as_str()).to_owned();
        Some(RepoInfo { owner, name })
    } else if repo.contains('/') {
        let mut parts = repo.splitn(2, '/');
        match (parts.next(), parts.next()) {
            (Some(owner), Some(name_part)) => Some(RepoInfo {
                owner: owner.to_owned(),
                name: strip_git_suffix(name_part).to_owned(),
            }),
            _ => None,
        }
    } else {
        None
    }
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

fn parse_reference(
    input: &str,
    default_repo: DefaultRepo,
    resource_type: ResourceType,
) -> Result<(RepoInfo, u64), VkError> {
    if let Some(res) = parse_github_url(input, resource_type) {
        return res;
    }
    if let Ok(number) = input.parse::<u64>() {
        let repo = default_repo
            .as_option()
            .and_then(parse_repo_str)
            .or_else(repo_from_fetch_head)
            .ok_or(VkError::RepoNotFound)?;
        return Ok((repo, number));
    }
    Err(VkError::InvalidRef)
}

pub fn parse_issue_reference<'a>(
    input: &str,
    default_repo: impl Into<DefaultRepo<'a>>,
) -> Result<(RepoInfo, u64), VkError> {
    parse_reference(input, default_repo.into(), ResourceType::Issues)
}

pub fn parse_pr_reference<'a>(
    input: &str,
    default_repo: impl Into<DefaultRepo<'a>>,
) -> Result<(RepoInfo, u64), VkError> {
    parse_reference(input, default_repo.into(), ResourceType::PullRequest)
}

/// Parse a pull request reference with an optional discussion fragment.
///
/// Accepts either a full GitHub URL or a bare number (using `default_repo`),
/// and an optional `#discussion_r` fragment. Returns the repository, pull
/// request number, and `Some(comment_id)` when a valid fragment is present.
///
/// # Examples
///
/// ```
/// # use vk::ref_parser::parse_pr_thread_reference;
/// let (repo, number, comment) = parse_pr_thread_reference("https://github.com/o/r/pull/1#discussion_r2", None)
///     .expect("valid reference");
/// assert_eq!(repo.owner, "o");
/// assert_eq!(repo.name, "r");
/// assert_eq!(number, 1);
/// assert_eq!(comment, Some(2));
/// ```
///
/// # Errors
///
/// Returns [`VkError::InvalidRef`] when the fragment is present but empty or
/// non-numeric, or when the input is not a valid pull request reference.
pub fn parse_pr_thread_reference<'a>(
    input: &str,
    default_repo: impl Into<DefaultRepo<'a>>,
) -> Result<(RepoInfo, u64, Option<u64>), VkError> {
    let default_repo = default_repo.into();
    let (base, comment) = match input.split_once(DISCUSSION_FRAGMENT) {
        Some((base, id)) if !id.is_empty() => {
            let cid = id.parse().map_err(|_| VkError::InvalidRef)?;
            (base, Some(cid))
        }
        Some(_) => return Err(VkError::InvalidRef),
        None => (input, None),
    };
    let (repo, number) = parse_pr_reference(base, default_repo)?;
    Ok((repo, number, comment))
}

/// Check if input is a bare discussion fragment (e.g., `#discussion_r123`).
///
/// Returns `true` when the input starts with `#discussion_r`, indicating a
/// fragment-only reference that requires PR auto-detection.
///
/// # Examples
///
/// ```
/// # use vk::ref_parser::is_fragment_only;
/// assert!(is_fragment_only("#discussion_r123"));
/// assert!(!is_fragment_only("42#discussion_r123"));
/// assert!(!is_fragment_only("https://github.com/o/r/pull/1#discussion_r123"));
/// ```
pub fn is_fragment_only(input: &str) -> bool {
    input.starts_with(DISCUSSION_FRAGMENT)
}

/// Extract the comment ID from a fragment-only input.
///
/// # Examples
///
/// ```
/// # use vk::ref_parser::parse_fragment_only;
/// assert_eq!(parse_fragment_only("#discussion_r123").expect("valid fragment"), 123);
/// ```
///
/// # Errors
///
/// Returns [`VkError::InvalidRef`] if the fragment is malformed or the ID is
/// not a valid number.
pub fn parse_fragment_only(input: &str) -> Result<u64, VkError> {
    let id_str = input
        .strip_prefix(DISCUSSION_FRAGMENT)
        .ok_or(VkError::InvalidRef)?;
    if id_str.is_empty() {
        return Err(VkError::InvalidRef);
    }
    id_str.parse().map_err(|_| VkError::InvalidRef)
}

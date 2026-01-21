//! Parse pull request and issue references into repository and number pairs, optionally including discussion comment IDs.

use std::sync::LazyLock;

use regex::Regex;
use url::Url;

use crate::VkError;

mod git;
#[cfg(test)]
mod tests;

pub use git::{current_branch, repo_from_fetch_head, repo_from_origin};
#[cfg(test)]
pub(crate) use git::{current_branch_impl, repo_from_fetch_head_impl, repo_from_origin_impl};

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
    #[expect(dead_code, reason = "public API for callers to construct DefaultRepo")]
    pub fn new(repo: &'a str) -> Self {
        Self(Some(repo))
    }

    /// No default repository; will fall back to `FETCH_HEAD` for numeric refs.
    #[expect(dead_code, reason = "public API for callers to construct DefaultRepo")]
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
    } else {
        // Accept only short-form "owner/repo" with exactly one slash
        let slash_count = repo.chars().filter(|&c| c == '/').count();
        if slash_count != 1 {
            return None;
        }
        let mut parts = repo.splitn(2, '/');
        match (parts.next(), parts.next()) {
            (Some(owner), Some(name_part)) if !owner.is_empty() && !name_part.is_empty() => {
                Some(RepoInfo {
                    owner: owner.to_owned(),
                    name: strip_git_suffix(name_part).to_owned(),
                })
            }
            _ => None,
        }
    }
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

/// Parse an issue reference into repository and issue number.
///
/// Accepts either a full GitHub issue URL or a bare issue number (using
/// `default_repo` for context, falling back to `FETCH_HEAD`).
///
/// # Arguments
///
/// * `input` - Issue reference: full URL (`https://github.com/o/r/issues/42`)
///   or bare number (`42`).
/// * `default_repo` - Optional `owner/repo` string for bare number resolution.
///
/// # Examples
///
/// ```
/// # use vk::ref_parser::parse_issue_reference;
/// let (repo, number) = parse_issue_reference(
///     "https://github.com/owner/repo/issues/42",
///     None,
/// ).expect("valid issue URL");
/// assert_eq!(repo.owner, "owner");
/// assert_eq!(repo.name, "repo");
/// assert_eq!(number, 42);
/// ```
///
/// # Errors
///
/// Returns [`VkError::InvalidRef`] for malformed input,
/// [`VkError::RepoNotFound`] when a bare number is provided without a
/// resolvable repository, or [`VkError::WrongResourceType`] when the URL
/// points to a pull request instead of an issue.
pub fn parse_issue_reference<'a>(
    input: &str,
    default_repo: impl Into<DefaultRepo<'a>>,
) -> Result<(RepoInfo, u64), VkError> {
    parse_reference(input, default_repo.into(), ResourceType::Issues)
}

/// Parse a pull request reference into repository and PR number.
///
/// Accepts either a full GitHub pull request URL or a bare PR number (using
/// `default_repo` for context, falling back to `FETCH_HEAD`).
///
/// # Arguments
///
/// * `input` - PR reference: full URL (`https://github.com/o/r/pull/42`)
///   or bare number (`42`).
/// * `default_repo` - Optional `owner/repo` string for bare number resolution.
///
/// # Examples
///
/// ```
/// # use vk::ref_parser::parse_pr_reference;
/// let (repo, number) = parse_pr_reference(
///     "https://github.com/owner/repo/pull/42",
///     None,
/// ).expect("valid PR URL");
/// assert_eq!(repo.owner, "owner");
/// assert_eq!(repo.name, "repo");
/// assert_eq!(number, 42);
/// ```
///
/// # Errors
///
/// Returns [`VkError::InvalidRef`] for malformed input,
/// [`VkError::RepoNotFound`] when a bare number is provided without a
/// resolvable repository, or [`VkError::WrongResourceType`] when the URL
/// points to an issue instead of a pull request.
pub fn parse_pr_reference<'a>(
    input: &str,
    default_repo: impl Into<DefaultRepo<'a>>,
) -> Result<(RepoInfo, u64), VkError> {
    parse_reference(input, default_repo.into(), ResourceType::PullRequest)
}

/// Parse a discussion fragment from the input, returning the base string and optional comment ID.
///
/// If the input contains `#discussion_r`, splits at that point and parses the numeric ID.
/// Returns the base URL/reference and `Some(comment_id)` when a valid fragment is present,
/// or the original input and `None` when no fragment exists.
fn parse_discussion_fragment(input: &str) -> Result<(&str, Option<u64>), VkError> {
    match input.split_once(DISCUSSION_FRAGMENT) {
        Some((base, id)) if !id.is_empty() => {
            let cid = id.parse().map_err(|_| VkError::InvalidRef)?;
            Ok((base, Some(cid)))
        }
        Some(_) => Err(VkError::InvalidRef),
        None => Ok((input, None)),
    }
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
    let (base, comment) = parse_discussion_fragment(input)?;
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

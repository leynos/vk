//! Internal parsing utilities.
//!
//! Contains helper functions and types for parsing GitHub references that are
//! not part of the public API.

use std::sync::LazyLock;

use regex::Regex;
use url::Url;

use super::{DefaultRepo, RepoInfo, parse_repo_str};
use crate::VkError;

/// Match the owner and repository components of a GitHub URL or remote.
pub(super) static GITHUB_RE: LazyLock<Result<Regex, regex::Error>> =
    LazyLock::new(|| Regex::new(r"github\.com[/:](?P<owner>[^/]+)/(?P<repo>[^/]+)"));

/// Remove the optional `.git` suffix from a repository name.
pub(super) fn strip_git_suffix(name: &str) -> &str {
    name.strip_suffix(".git").unwrap_or(name)
}

/// Render a [`RepoInfo`] as `owner/name` for trace events.
fn format_repo(repo: &RepoInfo) -> String {
    format!("{}/{}", repo.owner, repo.name)
}

/// GitHub resource kind accepted by a reference parser.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum ResourceType {
    /// An issue reference.
    Issues,
    /// A pull-request reference.
    PullRequest,
}

impl ResourceType {
    /// Return URL path segments valid for this resource kind.
    pub(super) fn allowed_segments(self) -> &'static [&'static str] {
        match self {
            Self::Issues => &["issues", "issue"],
            Self::PullRequest => &["pull", "pulls"],
        }
    }
}

/// Parse a full GitHub URL for the requested resource kind.
pub(super) fn parse_github_url(
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

/// Parse a full URL or bare number into a repository and resource number.
pub(super) fn parse_reference(
    input: &str,
    default_repo: DefaultRepo,
    resource_type: ResourceType,
) -> Result<(RepoInfo, u64), VkError> {
    if let Some(res) = parse_github_url(input, resource_type) {
        return res;
    }
    if let Ok(number) = input.parse::<u64>() {
        // `origin` is a last-resort fallback: it covers fresh worktrees where
        // `FETCH_HEAD` has not yet been written. `FETCH_HEAD` still takes
        // precedence because in fork workflows it identifies the upstream
        // repository while `origin` points at the user's fork.
        //
        // Each `.inspect(...)` fires only when its `Option` is `Some`, so
        // exactly one `debug!` runs and it identifies the winning source.
        // Mirrors the instrumentation on `resolve_branch_and_repo` so
        // diagnostics for bare-number references stay symmetrical with
        // branch-based PR detection.
        let repo = default_repo
            .as_option()
            .and_then(parse_repo_str)
            .inspect(|r| {
                tracing::debug!(repo = %format_repo(r), "resolved repo from --repo");
            })
            .or_else(|| {
                super::repo_from_fetch_head().inspect(|r| {
                    tracing::debug!(repo = %format_repo(r), "resolved repo from FETCH_HEAD");
                })
            })
            .or_else(|| {
                super::repo_from_origin().inspect(|r| {
                    tracing::debug!(repo = %format_repo(r), "resolved repo from origin remote");
                })
            })
            .ok_or(VkError::RepoNotFound)?;
        return Ok((repo, number));
    }
    Err(VkError::InvalidRef)
}

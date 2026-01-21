//! Internal parsing utilities.
//!
//! Contains helper functions and types for parsing GitHub references that are
//! not part of the public API.

use std::sync::LazyLock;

use regex::Regex;
use url::Url;

use super::{DefaultRepo, RepoInfo, parse_repo_str};
use crate::VkError;

pub(super) static GITHUB_RE: LazyLock<Result<Regex, regex::Error>> =
    LazyLock::new(|| Regex::new(r"github\.com[/:](?P<owner>[^/]+)/(?P<repo>[^/]+)"));

pub(super) fn strip_git_suffix(name: &str) -> &str {
    name.strip_suffix(".git").unwrap_or(name)
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ResourceType {
    Issues,
    PullRequest,
}

impl ResourceType {
    pub(super) fn allowed_segments(self) -> &'static [&'static str] {
        match self {
            Self::Issues => &["issues", "issue"],
            Self::PullRequest => &["pull", "pulls"],
        }
    }
}

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

pub(super) fn parse_reference(
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
            .or_else(super::repo_from_fetch_head)
            .ok_or(VkError::RepoNotFound)?;
        return Ok((repo, number));
    }
    Err(VkError::InvalidRef)
}

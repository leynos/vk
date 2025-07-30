//! Helpers for parsing repository references.
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "docs omitted"
)]

use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

use crate::api::VkError;

#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub owner: String,
    pub name: String,
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
    Regex::new(r"github\.com[/:](?P<owner>[^/]+)/(?P<repo>[^/.]+)").expect("valid regex")
});

#[expect(
    clippy::result_large_err,
    reason = "VkError has many variants but they are small"
)]
fn parse_reference(
    input: &str,
    default_repo: Option<&str>,
    resource_type: ResourceType,
) -> Result<(RepoInfo, u64), VkError> {
    if let Ok(url) = url::Url::parse(input) {
        if url.host_str() == Some("github.com") {
            let segments_iter = url.path_segments().ok_or(VkError::InvalidRef)?;
            let segments: Vec<_> = segments_iter.collect();
            if segments.len() >= 4 {
                let segment = segments.get(2).expect("length checked");
                let allowed = resource_type.allowed_segments();
                if allowed.contains(segment) {
                    let owner = (*segments.first().expect("length checked")).to_owned();
                    let repo_segment = segments.get(1).expect("length checked");
                    let name = repo_segment
                        .strip_suffix(".git")
                        .unwrap_or(repo_segment)
                        .to_owned();
                    let number: u64 = segments
                        .get(3)
                        .expect("length checked")
                        .parse()
                        .map_err(|_| VkError::InvalidRef)?;
                    return Ok((RepoInfo { owner, name }, number));
                }
                return Err(VkError::WrongResourceType {
                    expected: allowed,
                    found: (*segment).to_owned(),
                });
            }
        }
        Err(VkError::InvalidRef)
    } else if let Ok(number) = input.parse::<u64>() {
        let repo = default_repo
            .and_then(repo_from_str)
            .or_else(repo_from_fetch_head)
            .ok_or(VkError::RepoNotFound)?;
        Ok((repo, number))
    } else {
        Err(VkError::InvalidRef)
    }
}

#[expect(
    clippy::result_large_err,
    reason = "VkError has many variants but they are small"
)]
pub fn parse_issue_reference(
    input: &str,
    default_repo: Option<&str>,
) -> Result<(RepoInfo, u64), VkError> {
    parse_reference(input, default_repo, ResourceType::Issues)
}

#[expect(
    clippy::result_large_err,
    reason = "VkError has many variants but they are small"
)]
pub fn parse_pr_reference(
    input: &str,
    default_repo: Option<&str>,
) -> Result<(RepoInfo, u64), VkError> {
    parse_reference(input, default_repo, ResourceType::PullRequest)
}

pub fn repo_from_fetch_head() -> Option<RepoInfo> {
    let path = Path::new(".git/FETCH_HEAD");
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        if let Some(caps) = GITHUB_RE.captures(line) {
            let owner = caps.name("owner")?.as_str().to_owned();
            let name_str = caps.name("repo")?.as_str();
            let name = name_str.strip_suffix(".git").unwrap_or(name_str).to_owned();
            return Some(RepoInfo { owner, name });
        }
    }
    None
}

pub fn repo_from_str(repo: &str) -> Option<RepoInfo> {
    if let Some(caps) = GITHUB_RE.captures(repo) {
        let owner = caps.name("owner")?.as_str().to_owned();
        let name = caps.name("repo")?.as_str().to_owned();
        Some(RepoInfo { owner, name })
    } else if repo.contains('/') {
        let mut parts = repo.splitn(2, '/');
        let owner = parts.next().expect("split ensured one slash");
        let name_part = parts.next().expect("split ensured two parts");
        Some(RepoInfo {
            owner: owner.to_owned(),
            name: name_part
                .strip_suffix(".git")
                .unwrap_or(name_part)
                .to_owned(),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parse_url() {
        let (repo, number) = parse_pr_reference("https://github.com/owner/repo/pull/42", None)
            .expect("valid reference");
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
        assert_eq!(number, 42);
    }

    #[test]
    fn parse_url_git_suffix() {
        let (repo, number) = parse_pr_reference("https://github.com/owner/repo.git/pull/7", None)
            .expect("valid reference");
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
        assert_eq!(number, 7);
    }

    #[test]
    fn parse_url_plural_segment() {
        let (repo, number) = parse_pr_reference("https://github.com/owner/repo/pulls/13", None)
            .expect("valid reference");
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
        assert_eq!(number, 13);
    }

    #[test]
    fn repo_from_fetch_head_git_suffix() {
        let dir = tempdir().expect("tempdir");
        let git_dir = dir.path().join(".git");
        fs::create_dir(&git_dir).expect("create git dir");
        fs::write(
            git_dir.join("FETCH_HEAD"),
            "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/foo/bar.git",
        )
        .expect("write FETCH_HEAD");
        let cwd = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(dir.path()).expect("chdir temp");
        let repo = repo_from_fetch_head().expect("repo from fetch head");
        std::env::set_current_dir(cwd).expect("restore cwd");
        assert_eq!(repo.owner, "foo");
        assert_eq!(repo.name, "bar");
    }

    #[test]
    fn repo_from_str_git_suffix() {
        let repo = repo_from_str("a/b.git").expect("parse repo");
        assert_eq!(repo.owner, "a");
        assert_eq!(repo.name, "b");
    }

    #[test]
    fn parse_issue_url() {
        let (repo, number) = parse_issue_reference("https://github.com/owner/repo/issues/3", None)
            .expect("valid ref");
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
        assert_eq!(number, 3);
    }

    #[test]
    fn parse_issue_url_plural() {
        let (repo, number) = parse_issue_reference("https://github.com/owner/repo/issues/31", None)
            .expect("valid ref");
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
        assert_eq!(number, 31);
    }

    #[test]
    fn parse_issue_url_git_suffix() {
        let (repo, number) =
            parse_issue_reference("https://github.com/owner/repo.git/issues/9", None)
                .expect("valid ref");
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
        assert_eq!(number, 9);
    }

    #[test]
    fn parse_issue_url_singular() {
        let (repo, number) = parse_issue_reference("https://github.com/owner/repo/issue/11", None)
            .expect("valid ref");
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
        assert_eq!(number, 11);
    }

    #[test]
    fn parse_pr_number_with_repo() {
        let (repo, number) = parse_pr_reference("5", Some("foo/bar")).expect("valid ref");
        assert_eq!(repo.owner, "foo");
        assert_eq!(repo.name, "bar");
        assert_eq!(number, 5);
    }

    #[test]
    fn parse_issue_number_with_repo() {
        let (repo, number) = parse_issue_reference("8", Some("baz/qux")).expect("valid ref");
        assert_eq!(repo.owner, "baz");
        assert_eq!(repo.name, "qux");
        assert_eq!(number, 8);
    }
}

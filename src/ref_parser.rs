//! Parse pull request and issue references into repository and number pairs, optionally including discussion comment IDs.

use crate::VkError;
use regex::Regex;
use std::sync::LazyLock;
use std::{fs, path::Path};
use url::Url;

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
    if parts.len() < 4 {
        return Some(Err(VkError::InvalidRef));
    }
    let segment = parts.get(2).expect("length checked");
    if !resource.allowed_segments().contains(segment) {
        return Some(Err(VkError::WrongResourceType {
            expected: resource.allowed_segments(),
            found: (*segment).into(),
        }));
    }
    let number_str = parts.get(3).expect("length checked");
    let Ok(number) = number_str.parse() else {
        return Some(Err(VkError::InvalidRef));
    };
    let owner = parts.first().expect("length checked");
    let repo_part = parts.get(1).expect("length checked");
    let repo = RepoInfo {
        owner: (*owner).into(),
        name: strip_git_suffix(repo_part).into(),
    };
    Some(Ok((repo, number)))
}

fn parse_repo_str(repo: &str) -> Option<RepoInfo> {
    if let Some(caps) = GITHUB_RE.captures(repo) {
        let owner = caps.name("owner")?.as_str().to_owned();
        let name = strip_git_suffix(caps.name("repo")?.as_str()).to_owned();
        Some(RepoInfo { owner, name })
    } else if repo.contains('/') {
        let mut parts = repo.splitn(2, '/');
        let owner = parts.next().expect("split ensured one slash");
        let name_part = parts.next().expect("split ensured two parts");
        Some(RepoInfo {
            owner: owner.to_owned(),
            name: strip_git_suffix(name_part).to_owned(),
        })
    } else {
        None
    }
}

fn repo_from_fetch_head() -> Option<RepoInfo> {
    let path = Path::new(".git/FETCH_HEAD");
    let content = fs::read_to_string(path).ok()?;
    content.lines().find_map(parse_repo_str)
}

fn parse_reference(
    input: &str,
    default_repo: Option<&str>,
    resource_type: ResourceType,
) -> Result<(RepoInfo, u64), VkError> {
    if let Some(res) = parse_github_url(input, resource_type) {
        return res;
    }
    if let Ok(number) = input.parse::<u64>() {
        let repo = default_repo
            .and_then(parse_repo_str)
            .or_else(repo_from_fetch_head)
            .ok_or(VkError::RepoNotFound)?;
        return Ok((repo, number));
    }
    Err(VkError::InvalidRef)
}

pub fn parse_issue_reference(
    input: &str,
    default_repo: Option<&str>,
) -> Result<(RepoInfo, u64), VkError> {
    parse_reference(input, default_repo, ResourceType::Issues)
}

pub fn parse_pr_reference(
    input: &str,
    default_repo: Option<&str>,
) -> Result<(RepoInfo, u64), VkError> {
    parse_reference(input, default_repo, ResourceType::PullRequest)
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
/// # use crate::ref_parser::parse_pr_thread_reference;
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
pub fn parse_pr_thread_reference(
    input: &str,
    default_repo: Option<&str>,
) -> Result<(RepoInfo, u64, Option<u64>), VkError> {
    const FRAG: &str = "#discussion_r";
    let (base, comment) = match input.split_once(FRAG) {
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
    fn parse_repo_str_git_suffix() {
        let repo = parse_repo_str("a/b.git").expect("parse repo");
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

    #[test]
    fn parse_pr_thread_reference_with_comment() {
        let (repo, number, comment) =
            parse_pr_thread_reference("https://github.com/owner/repo/pull/1#discussion_r99", None)
                .expect("parse");
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
        assert_eq!(number, 1);
        assert_eq!(comment, Some(99));
    }

    use rstest::rstest;

    #[rstest]
    #[case("https://github.com/o/r/pull/1#discussion_r")]
    #[case("https://github.com/o/r/pull/1#discussion_rabc")]
    fn parse_pr_thread_reference_rejects_bad_fragment(#[case] input: &str) {
        let err = parse_pr_thread_reference(input, None).expect_err("invalid ref");
        assert!(matches!(err, VkError::InvalidRef));
    }
}

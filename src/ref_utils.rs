//! Reference parsing utilities and shared GraphQL structures.
#![allow(
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    reason = "internal helpers"
)]

use clap::CommandFactory;
use figment::error::{Error as FigmentError, Kind as FigmentKind};
use ortho_config::{OrthoConfig, OrthoError, load_and_merge_subcommand_for};
use regex::Regex;
use serde::Deserialize;
use serde::Serialize;
use std::sync::LazyLock;
use std::{env, fs, path::Path};
use thiserror::Error;
use url::Url;

/// Repository owner and name pair.
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

/// Errors returned by library functions.
#[derive(Error, Debug)]
#[allow(clippy::module_name_repetitions, reason = "exported for tests")]
pub enum VkError {
    #[error("unable to determine repository")]
    RepoNotFound,
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("request failed when running {context}: {source}")]
    RequestContext {
        context: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("invalid reference")]
    InvalidRef,
    #[error("expected URL path segment in {expected:?}, found '{found}'")]
    WrongResourceType {
        expected: &'static [&'static str],
        found: String,
    },
    #[error("bad response: {0}")]
    BadResponse(String),
    #[error("malformed response: {0}")]
    BadResponseSerde(String),
    #[error("API errors: {0}")]
    ApiErrors(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("configuration error: {0}")]
    Config(#[from] ortho_config::OrthoError),
}

static GITHUB_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"github\.com[/:](?P<owner>[^/]+)/(?P<repo>[^/.]+)").expect("valid regex")
});

static UTF8_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bUTF-?8\b").expect("valid regex"));

#[derive(Debug, Deserialize)]
pub struct ThreadData {
    pub repository: Repository,
}

#[derive(Debug, Deserialize)]
pub struct Repository {
    #[serde(rename = "pullRequest")]
    pub pull_request: PullRequest,
}

#[derive(Debug, Deserialize)]
pub struct PullRequest {
    #[serde(rename = "reviewThreads")]
    pub review_threads: ReviewThreadConnection,
}

#[derive(Deserialize)]
pub struct IssueData {
    pub repository: IssueRepository,
}

#[derive(Deserialize)]
pub struct IssueRepository {
    pub issue: Issue,
}

#[derive(Deserialize)]
pub struct Issue {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct ReviewThreadConnection {
    pub nodes: Vec<ReviewThread>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Debug, Deserialize, Default)]
pub struct ReviewThread {
    pub id: String,
    #[serde(rename = "isResolved")]
    #[allow(
        dead_code,
        reason = "GraphQL query requires this field but it is unused"
    )]
    pub is_resolved: bool,
    pub comments: CommentConnection,
}

#[derive(Debug, Deserialize, Default)]
pub struct CommentConnection {
    pub nodes: Vec<ReviewComment>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Debug, Deserialize, Default)]
pub struct ReviewComment {
    pub body: String,
    #[serde(rename = "diffHunk")]
    pub diff_hunk: String,
    #[serde(rename = "originalPosition")]
    pub original_position: Option<i32>,
    pub position: Option<i32>,
    #[allow(dead_code, reason = "stored for completeness; not displayed yet")]
    pub path: String,
    pub url: String,
    pub author: Option<User>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PageInfo {
    #[serde(rename = "hasNextPage")]
    pub has_next_page: bool,
    #[serde(rename = "endCursor")]
    pub end_cursor: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct User {
    pub login: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct CommentNodeWrapper {
    pub node: Option<CommentNode>,
}

#[derive(Debug, Deserialize, Default)]
pub struct CommentNode {
    pub comments: CommentConnection,
}

#[expect(
    clippy::result_large_err,
    reason = "configuration loading errors can be verbose"
)]
pub fn load_with_reference_fallback<T>(cli_args: T) -> Result<T, OrthoError>
where
    T: OrthoConfig + Serialize + Default + CommandFactory + Clone,
{
    fn missing_reference(err: &FigmentError) -> bool {
        err.clone()
            .into_iter()
            .any(|e| matches!(e.kind, FigmentKind::MissingField(ref f) if f == "reference"))
    }

    match load_and_merge_subcommand_for::<T>(&cli_args) {
        Ok(v) => Ok(v),
        Err(OrthoError::Gathering(e)) => {
            if missing_reference(&e) {
                Ok(cli_args)
            } else {
                Err(OrthoError::Gathering(e))
            }
        }
        Err(e) => Err(e),
    }
}

#[expect(
    clippy::result_large_err,
    reason = "VkError has many variants but they are small"
)]
fn parse_reference(
    input: &str,
    default_repo: Option<&str>,
    resource_type: ResourceType,
) -> Result<(RepoInfo, u64), VkError> {
    if let Ok(url) = Url::parse(input) {
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

fn repo_from_fetch_head() -> Option<RepoInfo> {
    let path = Path::new(".git/FETCH_HEAD");
    let content = fs::read_to_string(path).ok()?;
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

fn repo_from_str(repo: &str) -> Option<RepoInfo> {
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

pub fn locale_is_utf8() -> bool {
    env::var("LC_ALL")
        .or_else(|_| env::var("LC_CTYPE"))
        .or_else(|_| env::var("LANG"))
        .map(|v| UTF8_RE.is_match(&v))
        .unwrap_or(false)
}

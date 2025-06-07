use clap::Parser;
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, USER_AGENT};
use serde::Deserialize;
use serde_json::json;
use std::{env, fs, path::Path};
use termimad::MadSkin;
use thiserror::Error;
use url::Url;

#[derive(Parser)]
#[command(name = "vk", about = "View Komments - show unresolved PR comments")]
struct Args {
    /// Pull request URL or number
    reference: String,
}

#[derive(Debug, Clone)]
struct RepoInfo {
    owner: String,
    name: String,
}

#[derive(Error, Debug)]
enum VkError {
    #[error("unable to determine repository")]
    RepoNotFound,
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("invalid reference")]
    InvalidRef,
    #[error("malformed response")]
    BadResponse,
    #[error("API errors: {0}")]
    ApiErrors(String),
}

static GITHUB_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"github\.com[/:](?P<owner>[^/]+)/(?P<repo>[^/.]+)").unwrap());

#[derive(Deserialize)]
struct GraphQlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: String,
}

#[derive(Deserialize)]
struct ThreadData {
    repository: Repository,
}

#[derive(Deserialize)]
struct Repository {
    #[serde(rename = "pullRequest")]
    pull_request: PullRequest,
}

#[derive(Deserialize)]
struct PullRequest {
    #[serde(rename = "reviewThreads")]
    review_threads: ReviewThreadConnection,
}

#[derive(Deserialize)]
struct ReviewThreadConnection {
    nodes: Vec<ReviewThread>,
}

#[derive(Deserialize)]
struct ReviewThread {
    #[serde(rename = "isResolved")]
    is_resolved: bool,
    comments: CommentConnection,
}

#[derive(Deserialize)]
struct CommentConnection {
    nodes: Vec<ReviewComment>,
}

#[derive(Deserialize)]
struct ReviewComment {
    body: String,
    url: String,
    author: Option<User>,
}

#[derive(Deserialize)]
struct User {
    login: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let (repo, number) = parse_reference(&args.reference)?;
    let token = env::var("GITHUB_TOKEN").unwrap_or_default();
    if token.is_empty() {
        eprintln!("warning: GITHUB_TOKEN not set, using anonymous API access");
    }

    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, "vk".parse().unwrap());
    headers.insert(ACCEPT, "application/vnd.github+json".parse().unwrap());
    if !token.is_empty() {
        headers.insert(AUTHORIZATION, format!("Bearer {}", token).parse().unwrap());
    }

    let query = r#"
        query($owner: String!, $name: String!, $number: Int!) {
          repository(owner: $owner, name: $name) {
            pullRequest(number: $number) {
              reviewThreads(first: 100) {
                nodes {
                  isResolved
                  comments(first: 100) {
                    nodes {
                      body
                      url
                      author { login }
                    }
                  }
                }
              }
            }
          }
        }
    "#;

    let client = reqwest::Client::new();
    let resp: GraphQlResponse<ThreadData> = client
        .post("https://api.github.com/graphql")
        .headers(headers)
        .json(&json!({
            "query": query,
            "variables": {
                "owner": repo.owner,
                "name": repo.name,
                "number": number,
            }
        }))
        .send()
        .await?
        .json()
        .await?;

    if let Some(errs) = resp.errors {
        let msg = errs
            .into_iter()
            .map(|e| e.message)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(VkError::ApiErrors(msg).into());
    }

    let threads = resp
        .data
        .ok_or(VkError::BadResponse)?
        .repository
        .pull_request
        .review_threads
        .nodes;

    let skin = MadSkin::default();
    for (i, t) in threads.iter().filter(|t| !t.is_resolved).enumerate() {
        println!("\n==================== Thread {} ====================\n", i + 1);
        for c in &t.comments.nodes {
            let user = c.author.as_ref().map_or("unknown", |u| u.login.as_str());
            println!("\n{} commented:\n", user);
            skin.print_text(&c.body);
            println!("{}", c.url);
        }
    }
    Ok(())
}

fn parse_reference(input: &str) -> Result<(RepoInfo, u64), VkError> {
    if let Ok(url) = Url::parse(input) {
        if url.host_str() == Some("github.com") {
            let segments: Vec<_> = url.path_segments().unwrap().collect();
            if segments.len() >= 4 && segments[2] == "pull" {
                let owner = segments[0].to_string();
                let name = segments[1]
                    .strip_suffix(".git")
                    .unwrap_or(segments[1])
                    .to_string();
                let number: u64 = segments[3].parse().map_err(|_| VkError::InvalidRef)?;
                return Ok((RepoInfo { owner, name }, number));
            }
        }
        Err(VkError::InvalidRef)
    } else if let Ok(number) = input.parse::<u64>() {
        let repo = repo_from_fetch_head()
            .or_else(repo_from_env)
            .ok_or(VkError::RepoNotFound)?;
        Ok((repo, number))
    } else {
        Err(VkError::InvalidRef)
    }
}

fn repo_from_fetch_head() -> Option<RepoInfo> {
    let path = Path::new(".git/FETCH_HEAD");
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        if let Some(caps) = GITHUB_RE.captures(line) {
            let owner = caps.name("owner")?.as_str().to_string();
            let name_str = caps.name("repo")?.as_str();
            let name = name_str
                .strip_suffix(".git")
                .unwrap_or(name_str)
                .to_string();
            return Some(RepoInfo { owner, name });
        }
    }
    None
}

fn repo_from_env() -> Option<RepoInfo> {
    let repo = env::var("VK_REPO").ok()?;
    if let Some(caps) = GITHUB_RE.captures(&repo) {
        let owner = caps.name("owner")?.as_str().to_string();
        let name = caps.name("repo")?.as_str().to_string();
        Some(RepoInfo { owner, name })
    } else if repo.contains('/') {
        let parts: Vec<_> = repo.splitn(2, '/').collect();
        Some(RepoInfo {
            owner: parts[0].to_string(),
            name: parts[1]
                .strip_suffix(".git")
                .unwrap_or(parts[1])
                .to_string(),
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
        let (repo, number) = parse_reference("https://github.com/owner/repo/pull/42").unwrap();
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
        assert_eq!(number, 42);
    }

    #[test]
    fn parse_url_git_suffix() {
        let (repo, number) = parse_reference("https://github.com/owner/repo.git/pull/7").unwrap();
        assert_eq!(repo.name, "repo");
        assert_eq!(number, 7);
    }

    #[test]
    fn repo_from_fetch_head_git_suffix() {
        let dir = tempdir().unwrap();
        let git_dir = dir.path().join(".git");
        fs::create_dir(&git_dir).unwrap();
        fs::write(
            git_dir.join("FETCH_HEAD"),
            "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/foo/bar.git",
        )
        .unwrap();
        let cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let repo = repo_from_fetch_head().unwrap();
        std::env::set_current_dir(cwd).unwrap();
        assert_eq!(repo.owner, "foo");
        assert_eq!(repo.name, "bar");
    }
}

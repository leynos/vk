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

fn handle_graphql_errors(errors: Vec<GraphQlError>) -> VkError {
    let msg = errors
        .into_iter()
        .map(|e| e.message)
        .collect::<Vec<_>>()
        .join(", ");
    VkError::ApiErrors(msg)
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
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
}

#[derive(Deserialize)]
struct ReviewThread {
    id: String,
    #[serde(rename = "isResolved")]
    is_resolved: bool,
    comments: CommentConnection,
}

#[derive(Deserialize)]
struct CommentConnection {
    nodes: Vec<ReviewComment>,
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
}

#[derive(Deserialize)]
struct ReviewComment {
    body: String,
    url: String,
    author: Option<User>,
}

#[derive(Deserialize)]
struct PageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
struct User {
    login: String,
}

#[derive(Deserialize)]
struct CommentNodeWrapper {
    node: Option<CommentNode>,
}

#[derive(Deserialize)]
struct CommentNode {
    comments: CommentConnection,
}

const THREADS_QUERY: &str = r#"
    query($owner: String!, $name: String!, $number: Int!, $cursor: String) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          reviewThreads(first: 100, after: $cursor, states: [UNRESOLVED]) {
            nodes {
              id
              isResolved
              comments(first: 100) {
                nodes {
                  body
                  url
                  author { login }
                }
                pageInfo { hasNextPage endCursor }
              }
            }
            pageInfo { hasNextPage endCursor }
          }
        }
      }
    }
"#;

const COMMENT_QUERY: &str = r#"
    query($id: ID!, $cursor: String) {
      node(id: $id) {
        ... on PullRequestReviewThread {
          comments(first: 100, after: $cursor) {
            nodes {
              body
              url
              author { login }
            }
            pageInfo { hasNextPage endCursor }
          }
        }
      }
    }
"#;

async fn paginate<T, F, Fut>(mut fetch: F) -> Result<Vec<T>, VkError>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<T>, PageInfo), VkError>>,
{
    let mut items = Vec::new();
    let mut cursor = None;
    loop {
        let (mut page, info) = fetch(cursor.clone()).await?;
        items.append(&mut page);
        if !info.has_next_page {
            break;
        }
        cursor = info.end_cursor;
    }
    Ok(items)
}

async fn fetch_comment_page(
    client: &reqwest::Client,
    headers: &HeaderMap,
    id: &str,
    cursor: Option<String>,
) -> Result<(Vec<ReviewComment>, PageInfo), VkError> {
    let resp: GraphQlResponse<CommentNodeWrapper> = client
        .post("https://api.github.com/graphql")
        .headers(headers.clone())
        .json(&json!({
            "query": COMMENT_QUERY,
            "variables": { "id": id, "cursor": cursor },
        }))
        .send()
        .await?
        .json()
        .await?;

    if let Some(errs) = resp.errors {
        return Err(handle_graphql_errors(errs));
    }
    let wrapper = resp.data.ok_or(VkError::BadResponse)?;
    let conn = wrapper.node.ok_or(VkError::BadResponse)?.comments;
    Ok((conn.nodes, conn.page_info))
}

async fn fetch_thread_page(
    client: &reqwest::Client,
    headers: &HeaderMap,
    repo: &RepoInfo,
    number: u64,
    cursor: Option<String>,
) -> Result<(Vec<ReviewThread>, PageInfo), VkError> {
    let resp: GraphQlResponse<ThreadData> = client
        .post("https://api.github.com/graphql")
        .headers(headers.clone())
        .json(&json!({
            "query": THREADS_QUERY,
            "variables": {
                "owner": repo.owner,
                "name": repo.name,
                "number": number,
                "cursor": cursor,
            }
        }))
        .send()
        .await?
        .json()
        .await?;

    if let Some(errs) = resp.errors {
        return Err(handle_graphql_errors(errs));
    }
    let data = resp.data.ok_or(VkError::BadResponse)?;
    let conn = data.repository.pull_request.review_threads;
    Ok((conn.nodes, conn.page_info))
}

async fn fetch_review_threads(
    client: &reqwest::Client,
    headers: &HeaderMap,
    repo: &RepoInfo,
    number: u64,
) -> Result<Vec<ReviewThread>, VkError> {
    let mut threads =
        paginate(|cursor| fetch_thread_page(client, headers, repo, number, cursor)).await?;

    for thread in &mut threads {
        let initial = std::mem::replace(
            &mut thread.comments,
            CommentConnection {
                nodes: Vec::new(),
                page_info: PageInfo {
                    has_next_page: false,
                    end_cursor: None,
                },
            },
        );
        let mut comments = initial.nodes;
        if initial.page_info.has_next_page {
            let more = paginate(|c| fetch_comment_page(client, headers, &thread.id, c)).await?;
            comments.extend(more);
        }
        thread.comments = CommentConnection {
            nodes: comments,
            page_info: PageInfo {
                has_next_page: false,
                end_cursor: None,
            },
        };
    }
    Ok(threads)
}

fn build_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, "vk".parse().unwrap());
    headers.insert(ACCEPT, "application/vnd.github+json".parse().unwrap());
    if !token.is_empty() {
        headers.insert(AUTHORIZATION, format!("Bearer {}", token).parse().unwrap());
    }
    headers
}

async fn run(args: Args) -> Result<(), VkError> {
    let (repo, number) = parse_reference(&args.reference)?;
    let token = env::var("GITHUB_TOKEN").unwrap_or_default();
    if token.is_empty() {
        eprintln!("warning: GITHUB_TOKEN not set, using anonymous API access");
    }

    let headers = build_headers(&token);
    let client = reqwest::Client::new();
    let threads = fetch_review_threads(&client, &headers, &repo, number).await?;
    if threads.is_empty() {
        println!("No unresolved comments.");
        return Ok(());
    }

    let skin = MadSkin::default();
    for t in threads {
        if t.is_resolved {
            continue;
        }
        for c in &t.comments.nodes {
            let user = c.author.as_ref().map_or("unknown", |u| u.login.as_str());
            println!("\n{} commented:\n", user);
            skin.print_text(&c.body);
            println!("{}", c.url);
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), VkError> {
    run(Args::parse()).await
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
        assert_eq!(repo.owner, "owner");
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

    #[test]
    fn repo_from_env_git_suffix() {
        unsafe { std::env::set_var("VK_REPO", "a/b.git") };
        let repo = repo_from_env().unwrap();
        assert_eq!(repo.owner, "a");
        assert_eq!(repo.name, "b");
        unsafe { std::env::remove_var("VK_REPO") };
    }
}

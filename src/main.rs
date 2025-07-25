//! Entry point for the `vk` command line tool.
//!
//! `vk` fetches unresolved review comments from GitHub's GraphQL API,
//! summarizing them by file before printing each thread. When a thread has
//! multiple comments on the same diff, the diff is shown only once.
//! After all comments are printed, the tool displays an `end of code review`
//! banner so calling processes know the output has finished.
mod cli_args;
mod reviews;
use crate::cli_args::{GlobalArgs, IssueArgs, PrArgs};
use crate::reviews::{fetch_reviews, latest_reviews, print_reviews};
use clap::{Parser, Subcommand};
use figment::error::{Error as FigmentError, Kind as FigmentKind};
use ortho_config::{OrthoConfig, OrthoError, load_and_merge_subcommand_for};
use regex::Regex;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, USER_AGENT};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use std::sync::LazyLock;
use std::{env, fs, path::Path};
use termimad::MadSkin;
use thiserror::Error;
use url::Url;

#[derive(Subcommand, Deserialize, Serialize, Clone, Debug)]
enum Commands {
    /// Show unresolved pull request comments
    Pr(PrArgs),
    /// Read a GitHub issue (todo)
    Issue(IssueArgs),
}

#[derive(Parser)]
#[command(
    name = "vk",
    about = "View Komments - show unresolved PR comments",
    subcommand_required = true,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: crate::Commands,
    #[command(flatten)]
    global: GlobalArgs,
}

#[derive(Debug, Clone)]
struct RepoInfo {
    owner: String,
    name: String,
}

#[derive(Clone, Copy)]
enum ResourceType {
    Issues,
    PullRequest,
}

impl ResourceType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Issues => "issues",
            Self::PullRequest => "pull",
        }
    }
}

#[derive(Error, Debug)]
enum VkError {
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
    #[error("expected URL path segment '{expected}', found '{found}'")]
    WrongResourceType {
        expected: &'static str,
        found: String,
    },
    #[error("malformed response")]
    BadResponse,
    #[error("API errors: {0}")]
    ApiErrors(String),
    #[error("configuration error: {0}")]
    Config(#[from] ortho_config::OrthoError),
}

static GITHUB_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"github\.com[/:](?P<owner>[^/]+)/(?P<repo>[^/.]+)").expect("valid regex")
});

static UTF8_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bUTF-?8\b").expect("valid regex"));
static HUNK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"@@ -(?P<old>\d+)(?:,(?P<old_count>\d+))? \+(?P<new>\d+)(?:,(?P<new_count>\d+))? @@",
    )
    .expect("valid regex")
});

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

struct GraphQLClient {
    client: reqwest::Client,
    headers: HeaderMap,
    endpoint: String,
}

impl GraphQLClient {
    fn new(token: &str) -> Self {
        Self::with_endpoint(token, GITHUB_GRAPHQL_URL)
    }

    fn with_endpoint(token: &str, endpoint: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            headers: build_headers(token),
            endpoint: endpoint.to_string(),
        }
    }

    async fn run_query<V, T>(&self, query: &str, variables: V) -> Result<T, VkError>
    where
        V: serde::Serialize,
        T: DeserializeOwned,
    {
        let payload = json!({ "query": query, "variables": &variables });
        let ctx = serde_json::to_string(&payload).unwrap_or_default();
        let resp: GraphQlResponse<T> = self
            .client
            .post(&self.endpoint)
            .headers(self.headers.clone())
            .json(&payload)
            .send()
            .await
            .map_err(|e| VkError::RequestContext {
                context: ctx.clone(),
                source: e,
            })?
            .json()
            .await
            .map_err(|e| VkError::RequestContext {
                context: ctx,
                source: e,
            })?;

        if let Some(errs) = resp.errors {
            return Err(handle_graphql_errors(errs));
        }
        resp.data.ok_or(VkError::BadResponse)
    }
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
struct IssueData {
    repository: IssueRepository,
}

#[derive(Deserialize)]
struct IssueRepository {
    issue: Issue,
}

#[derive(Deserialize)]
struct Issue {
    title: String,
    body: String,
}

#[derive(Debug, Deserialize, Default)]
struct ReviewThreadConnection {
    nodes: Vec<ReviewThread>,
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
}

#[derive(Debug, Deserialize, Default)]
struct ReviewThread {
    id: String,
    #[serde(rename = "isResolved")]
    #[allow(
        dead_code,
        reason = "GraphQL query requires this field but it is unused"
    )]
    is_resolved: bool,
    comments: CommentConnection,
}

#[derive(Debug, Deserialize, Default)]
struct CommentConnection {
    nodes: Vec<ReviewComment>,
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
}

#[derive(Debug, Deserialize, Default)]
struct ReviewComment {
    body: String,
    #[serde(rename = "diffHunk")]
    diff_hunk: String,
    #[serde(rename = "originalPosition")]
    original_position: Option<i32>,
    position: Option<i32>,
    #[allow(dead_code, reason = "stored for completeness; not displayed yet")]
    path: String,
    url: String,
    author: Option<User>,
}

#[derive(Debug, Deserialize, Default)]
struct PageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct User {
    login: String,
}

#[derive(Debug, Deserialize, Default)]
struct CommentNodeWrapper {
    node: Option<CommentNode>,
}

#[derive(Debug, Deserialize, Default)]
struct CommentNode {
    comments: CommentConnection,
}

const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";

/// Width of the line number gutter in diff output
const GUTTER_WIDTH: usize = 5;

const THREADS_QUERY: &str = r"
    query($owner: String!, $name: String!, $number: Int!, $cursor: String) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          reviewThreads(first: 100, after: $cursor) {
            nodes {
              id
              isResolved
              comments(first: 100) {
                nodes {
                  body
                  diffHunk
                  originalPosition
                  position
                  path
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
";

const COMMENT_QUERY: &str = r"
    query($id: ID!, $cursor: String) {
      node(id: $id) {
        ... on PullRequestReviewThread {
          comments(first: 100, after: $cursor) {
            nodes {
              body
              diffHunk
              originalPosition
              position
              path
              url
              author { login }
            }
            pageInfo { hasNextPage endCursor }
          }
        }
      }
    }
";

const ISSUE_QUERY: &str = r"
    query($owner: String!, $name: String!, $number: Int!) {
      repository(owner: $owner, name: $name) {
        issue(number: $number) {
          title
          body
        }
      }
    }
";

pub(crate) async fn paginate<T, F, Fut>(mut fetch: F) -> Result<Vec<T>, VkError>
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
    client: &GraphQLClient,
    id: &str,
    cursor: Option<String>,
) -> Result<(Vec<ReviewComment>, PageInfo), VkError> {
    let wrapper: CommentNodeWrapper = client
        .run_query(COMMENT_QUERY, json!({ "id": id, "cursor": cursor }))
        .await?;
    let conn = wrapper.node.ok_or(VkError::BadResponse)?.comments;
    Ok((conn.nodes, conn.page_info))
}

async fn fetch_issue(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Issue, VkError> {
    let data: IssueData = client
        .run_query(
            ISSUE_QUERY,
            json!({
                "owner": repo.owner.as_str(),
                "name": repo.name.as_str(),
                "number": number
            }),
        )
        .await?;
    Ok(data.repository.issue)
}

async fn fetch_thread_page(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
    cursor: Option<String>,
) -> Result<(Vec<ReviewThread>, PageInfo), VkError> {
    let data: ThreadData = client
        .run_query(
            THREADS_QUERY,
            json!({
                "owner": repo.owner.as_str(),
                "name": repo.name.as_str(),
                "number": number,
                "cursor": cursor,
            }),
        )
        .await?;
    let conn = data.repository.pull_request.review_threads;
    Ok((conn.nodes, conn.page_info))
}

async fn fetch_review_threads(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Vec<ReviewThread>, VkError> {
    let mut threads = paginate(|cursor| fetch_thread_page(client, repo, number, cursor)).await?;
    threads.retain(|t| !t.is_resolved);

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
            let more = paginate(|c| fetch_comment_page(client, &thread.id, c)).await?;
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

fn format_comment_diff(comment: &ReviewComment) -> Result<String, std::fmt::Error> {
    use std::fmt::Write;

    fn parse_diff_lines<'a, I>(
        lines: I,
        mut old_line: Option<i32>,
        mut new_line: Option<i32>,
    ) -> Vec<(Option<i32>, Option<i32>, String)>
    where
        I: Iterator<Item = &'a str>,
    {
        let mut parsed = Vec::new();
        for l in lines {
            if l.starts_with('+') {
                parsed.push((None, new_line, l.to_owned()));
                if let Some(ref mut n) = new_line {
                    *n += 1;
                }
            } else if l.starts_with('-') {
                parsed.push((old_line, None, l.to_owned()));
                if let Some(ref mut o) = old_line {
                    *o += 1;
                }
            } else {
                let text = l.strip_prefix(' ').unwrap_or(l);
                parsed.push((old_line, new_line, format!(" {text}")));
                if let Some(ref mut o) = old_line {
                    *o += 1;
                }
                if let Some(ref mut n) = new_line {
                    *n += 1;
                }
            }
        }
        parsed
    }

    fn num_disp(num: i32) -> String {
        let mut s = num.to_string();
        if s.len() > GUTTER_WIDTH {
            let start = s.len() - GUTTER_WIDTH;
            s = s.split_off(start);
        }
        format!("{s:>GUTTER_WIDTH$}")
    }

    let mut lines_iter = comment.diff_hunk.lines();
    let Some(header) = lines_iter.next() else {
        return Ok(String::new());
    };

    let lines: Vec<(Option<i32>, Option<i32>, String)> = HUNK_RE.captures(header).map_or_else(
        || parse_diff_lines(comment.diff_hunk.lines(), None, None),
        |caps| {
            let old_start: i32 = caps
                .name("old")
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            let new_start: i32 = caps
                .name("new")
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(0);
            let _old_count: usize = caps
                .name("old_count")
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);
            let _new_count: usize = caps
                .name("new_count")
                .and_then(|m| m.as_str().parse().ok())
                .unwrap_or(1);

            parse_diff_lines(lines_iter, Some(old_start), Some(new_start))
        },
    );

    let target_idx = lines
        .iter()
        .position(|(o, n, _)| comment.original_position == *o || comment.position == *n);
    let (start, end) = target_idx.map_or_else(
        || (0, std::cmp::min(lines.len(), 20)),
        |idx| (idx.saturating_sub(5), std::cmp::min(lines.len(), idx + 6)),
    );

    let mut out = String::new();
    for (o, n, text) in lines.get(start..end).unwrap_or(&[]) {
        // Prefer the new line number, fall back to old, or blanks if neither
        let disp = n.or(*o).map_or_else(|| " ".repeat(GUTTER_WIDTH), num_disp);

        writeln!(&mut out, "{disp}|{text}")?;
    }
    Ok(out)
}

/// Format the body of a single review comment.
///
/// The formatted output includes the author's login in bold followed by the
/// markdown-rendered comment text and a trailing newline.
///
/// * `out` - Destination implementing [`Write`]
/// * `skin` - Skin used for markdown formatting
/// * `comment` - Review comment to display
fn write_comment_body<W: std::io::Write>(
    mut out: W,
    skin: &MadSkin,
    comment: &ReviewComment,
) -> anyhow::Result<()> {
    let author = comment.author.as_ref().map_or("", |u| u.login.as_str());
    writeln!(out, "\u{1f4ac}  \x1b[1m{author}\x1b[0m wrote:")?;
    let _ = skin.write_text_on(&mut out, &comment.body);
    writeln!(out)?;
    Ok(())
}

/// Print a single review comment including its diff hunk.
///
/// The diff is emitted first, followed by the comment body formatted using
/// [`write_comment_body`].
///
/// * `out` - Destination implementing [`Write`]
/// * `skin` - Skin used for markdown formatting
/// * `comment` - Review comment to display
fn write_comment<W: std::io::Write>(
    mut out: W,
    skin: &MadSkin,
    comment: &ReviewComment,
) -> anyhow::Result<()> {
    let diff = format_comment_diff(comment)?;
    write!(out, "{diff}")?;
    write_comment_body(&mut out, skin, comment)?;
    Ok(())
}

/// Write all comments in a review thread, showing the diff only once.
///
/// The first comment is printed via [`write_comment`]. Subsequent comments omit
/// the diff and are printed with [`write_comment_body`]. Each comment URL is
/// appended on its own line.
///
/// * `out` - Destination implementing [`Write`]
/// * `skin` - Skin used for markdown formatting
/// * `thread` - Review thread to display
fn write_thread<W: std::io::Write>(
    mut out: W,
    skin: &MadSkin,
    thread: &ReviewThread,
) -> anyhow::Result<()> {
    let mut iter = thread.comments.nodes.iter();
    if let Some(first) = iter.next() {
        write_comment(&mut out, skin, first)?;
        writeln!(out, "{}", first.url)?;
        for c in iter {
            write_comment_body(&mut out, skin, c)?;
            writeln!(out, "{}", c.url)?;
        }
    }
    Ok(())
}

/// Print a review thread to stdout.
///
/// This simply calls [`write_thread`] with a locked `stdout` handle.
fn print_thread(skin: &MadSkin, thread: &ReviewThread) -> anyhow::Result<()> {
    write_thread(std::io::stdout().lock(), skin, thread)
}

fn summarize_files(threads: &[ReviewThread]) -> Vec<(String, usize)> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for t in threads {
        for c in &t.comments.nodes {
            *counts.entry(c.path.clone()).or_default() += 1;
        }
    }
    counts.into_iter().collect()
}

fn write_summary<W: std::io::Write>(
    mut out: W,
    summary: &[(String, usize)],
) -> std::io::Result<()> {
    if summary.is_empty() {
        return Ok(());
    }
    writeln!(out, "Summary:")?;
    for (path, count) in summary {
        let label = if *count == 1 { "comment" } else { "comments" };
        writeln!(out, "{path}: {count} {label}")?;
    }
    writeln!(out)?;
    Ok(())
}

fn print_summary(summary: &[(String, usize)]) {
    let _ = write_summary(std::io::stdout().lock(), summary);
}

/// Print a closing banner once all review threads have been displayed.
fn print_end_banner() {
    println!("========== end of code review ==========");
}

fn build_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, "vk".parse().expect("static string"));
    headers.insert(
        ACCEPT,
        "application/vnd.github+json"
            .parse()
            .expect("static string"),
    );
    if !token.is_empty() {
        headers.insert(
            AUTHORIZATION,
            format!("Bearer {token}").parse().expect("valid header"),
        );
    }
    headers
}

#[allow(
    clippy::result_large_err,
    reason = "VkError has many variants but they are small"
)]
async fn run_pr(args: PrArgs, repo: Option<&str>) -> Result<(), VkError> {
    let reference = args.reference.as_deref().ok_or(VkError::InvalidRef)?;
    let (repo, number) = parse_pr_reference(reference, repo)?;
    let token = env::var("GITHUB_TOKEN").unwrap_or_default();
    if token.is_empty() {
        eprintln!("warning: GITHUB_TOKEN not set, using anonymous API access");
    }
    if !locale_is_utf8() {
        eprintln!("warning: terminal locale is not UTF-8; emojis may not render correctly");
    }

    let client = GraphQLClient::new(&token);
    let threads = fetch_review_threads(&client, &repo, number).await?;
    let reviews = fetch_reviews(&client, &repo, number).await?;
    if threads.is_empty() {
        println!("No unresolved comments.");
        return Ok(());
    }

    let summary = summarize_files(&threads);
    print_summary(&summary);

    let skin = MadSkin::default();
    let latest = latest_reviews(reviews);
    print_reviews(&skin, &latest);

    for t in threads {
        if let Err(e) = print_thread(&skin, &t) {
            eprintln!("error printing thread: {e}");
        }
    }
    print_end_banner();
    Ok(())
}

#[allow(
    clippy::result_large_err,
    reason = "VkError has many variants but they are small"
)]
async fn run_issue(args: IssueArgs, repo: Option<&str>) -> Result<(), VkError> {
    let reference = args.reference.as_deref().ok_or(VkError::InvalidRef)?;
    let (repo, number) = parse_issue_reference(reference, repo)?;
    let token = env::var("GITHUB_TOKEN").unwrap_or_default();
    if token.is_empty() {
        eprintln!("warning: GITHUB_TOKEN not set, using anonymous API access");
    }
    if !locale_is_utf8() {
        eprintln!("warning: terminal locale is not UTF-8; emojis may not render correctly");
    }

    let client = GraphQLClient::new(&token);
    let issue = fetch_issue(&client, &repo, number).await?;

    let skin = MadSkin::default();
    println!("\x1b[1m{}\x1b[0m", issue.title);
    skin.print_text(&issue.body);
    println!();
    Ok(())
}

fn missing_reference(err: &FigmentError) -> bool {
    err.clone()
        .into_iter()
        .any(|e| matches!(e.kind, FigmentKind::MissingField(ref f) if f == "reference"))
}
#[expect(
    clippy::result_large_err,
    reason = "configuration loading errors can be verbose"
)]
fn load_with_reference_fallback<T>(cli_args: T) -> Result<T, OrthoError>
where
    T: OrthoConfig + serde::Serialize + Default + clap::CommandFactory + Clone,
{
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

#[tokio::main]
#[allow(
    clippy::result_large_err,
    reason = "VkError has many variants but they are small"
)]
async fn main() -> Result<(), VkError> {
    let cli = Cli::parse();
    let mut global = GlobalArgs::load_from_iter(std::env::args_os().take(1))?;
    global.merge(cli.global);
    match cli.command {
        Commands::Pr(pr_cli) => {
            let args = load_with_reference_fallback::<PrArgs>(pr_cli.clone())?;
            run_pr(args, global.repo.as_deref()).await
        }
        Commands::Issue(issue_cli) => {
            let args = load_with_reference_fallback::<IssueArgs>(issue_cli.clone())?;
            run_issue(args, global.repo.as_deref()).await
        }
    }
}

#[allow(
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
                if segments.get(2).expect("length checked") == &resource_type.as_str() {
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
                    expected: resource_type.as_str(),
                    found: (*segments.get(2).expect("length checked")).to_owned(),
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

#[allow(
    clippy::result_large_err,
    reason = "VkError has many variants but they are small"
)]
fn parse_issue_reference(
    input: &str,
    default_repo: Option<&str>,
) -> Result<(RepoInfo, u64), VkError> {
    parse_reference(input, default_repo, ResourceType::Issues)
}

#[allow(
    clippy::result_large_err,
    reason = "VkError has many variants but they are small"
)]
fn parse_pr_reference(input: &str, default_repo: Option<&str>) -> Result<(RepoInfo, u64), VkError> {
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

fn locale_is_utf8() -> bool {
    env::var("LC_ALL")
        .or_else(|_| env::var("LC_CTYPE"))
        .or_else(|_| env::var("LANG"))
        .map(|v| UTF8_RE.is_match(&v))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;
    use std::fmt::Write;
    use std::fs;
    use tempfile::tempdir;

    fn set_var<K: AsRef<std::ffi::OsStr>, V: AsRef<std::ffi::OsStr>>(key: K, value: V) {
        // SAFETY: manipulating environment variables in tests is safe because tests run serially.
        unsafe { std::env::set_var(key, value) }
    }

    fn remove_var<K: AsRef<std::ffi::OsStr>>(key: K) {
        // SAFETY: manipulating environment variables in tests is safe because tests run serially.
        unsafe { std::env::remove_var(key) }
    }

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
    fn cli_loads_repo_from_flag() {
        let cli = Cli::try_parse_from(["vk", "--repo", "foo/bar", "pr", "1"]).expect("parse cli");
        assert_eq!(cli.global.repo.as_deref(), Some("foo/bar"));
    }

    use serial_test::serial;

    #[test]
    #[serial]
    fn detect_utf8_locale() {
        let old_all = std::env::var("LC_ALL").ok();
        let old_ctype = std::env::var("LC_CTYPE").ok();
        let old_lang = std::env::var("LANG").ok();

        set_var("LC_ALL", "en_GB.UTF-8");
        remove_var("LC_CTYPE");
        remove_var("LANG");
        assert!(locale_is_utf8());

        set_var("LC_ALL", "en_GB.UTF8");
        assert!(locale_is_utf8());

        set_var("LC_ALL", "en_GB.utf8");
        assert!(locale_is_utf8());

        set_var("LC_ALL", "en_GB.UTF80");
        assert!(!locale_is_utf8());

        remove_var("LC_ALL");
        set_var("LC_CTYPE", "en_GB.UTF-8");
        assert!(locale_is_utf8());

        set_var("LC_CTYPE", "C");
        assert!(!locale_is_utf8());

        remove_var("LC_CTYPE");
        set_var("LANG", "en_GB.UTF-8");
        assert!(locale_is_utf8());

        set_var("LANG", "C");
        assert!(!locale_is_utf8());

        match old_all {
            Some(v) => set_var("LC_ALL", v),
            None => remove_var("LC_ALL"),
        }
        match old_ctype {
            Some(v) => set_var("LC_CTYPE", v),
            None => remove_var("LC_CTYPE"),
        }
        match old_lang {
            Some(v) => set_var("LANG", v),
            None => remove_var("LANG"),
        }
    }

    #[test]
    fn format_comment_diff_sample() {
        let data = fs::read_to_string("tests/fixtures/review_comment.json").expect("fixture");
        let comment: ReviewComment = serde_json::from_str(&data).expect("deserialize");
        let diff = format_comment_diff(&comment).expect("diff");
        assert!(diff.contains("-import dataclasses"));
        assert!(diff.contains("import typing"));
    }

    #[test]
    fn hunk_re_variants() {
        let caps = HUNK_RE.captures("@@ -1 +2 @@").expect("regex");
        assert_eq!(&caps["old"], "1");
        assert!(caps.name("old_count").is_none());
        assert_eq!(&caps["new"], "2");
        assert!(caps.name("new_count").is_none());

        let caps = HUNK_RE.captures("@@ -3,4 +5 @@").expect("regex");
        assert_eq!(&caps["old"], "3");
        assert_eq!(caps.name("old_count").expect("old count").as_str(), "4");
        assert_eq!(&caps["new"], "5");
        assert!(caps.name("new_count").is_none());

        let caps = HUNK_RE.captures("@@ -7 +8,2 @@").expect("regex");
        assert_eq!(&caps["old"], "7");
        assert!(caps.name("old_count").is_none());
        assert_eq!(&caps["new"], "8");
        assert_eq!(caps.name("new_count").expect("new count").as_str(), "2");
    }

    #[test]
    fn format_comment_diff_invalid_header() {
        let comment = ReviewComment {
            body: String::new(),
            diff_hunk: "not a hunk\n-line1\n+line1".to_string(),
            original_position: None,
            position: None,
            path: String::new(),
            url: String::new(),
            author: None,
        };
        let out = format_comment_diff(&comment).expect("diff");
        assert!(out.contains("-line1"));
        assert!(out.contains("+line1"));
    }

    #[test]
    fn format_comment_diff_caps_output() {
        let mut diff = String::from("@@ -1,30 +1,30 @@\n");
        for i in 0..30 {
            writeln!(&mut diff, " line{i}").expect("write diff line");
        }
        let comment = ReviewComment {
            body: String::new(),
            diff_hunk: diff,
            original_position: None,
            position: None,
            path: String::new(),
            url: String::new(),
            author: None,
        };
        let out = format_comment_diff(&comment).expect("diff");
        assert_eq!(out.lines().count(), 20);
    }

    #[test]
    fn cli_requires_subcommand() {
        assert!(Cli::try_parse_from(["vk"]).is_err());
    }

    #[test]
    fn pr_subcommand_parses() {
        let cli = Cli::try_parse_from(["vk", "pr", "123"]).expect("parse cli");
        match cli.command {
            Commands::Pr(args) => assert_eq!(args.reference.as_deref(), Some("123")),
            Commands::Issue(_) => panic!("wrong variant"),
        }
    }

    #[test]
    fn issue_subcommand_parses() {
        let cli = Cli::try_parse_from(["vk", "issue", "123"]).expect("parse cli");
        match cli.command {
            Commands::Issue(args) => assert_eq!(args.reference.as_deref(), Some("123")),
            Commands::Pr(_) => panic!("wrong variant"),
        }
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
    fn parse_issue_url_git_suffix() {
        let (repo, number) =
            parse_issue_reference("https://github.com/owner/repo.git/issues/9", None)
                .expect("valid ref");
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
        assert_eq!(number, 9);
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

    #[fixture]
    fn review_comment(#[default("test.rs")] path: &str) -> ReviewComment {
        ReviewComment {
            path: path.into(),
            ..Default::default()
        }
    }

    #[rstest]
    #[case(vec![], vec![])]
    #[case(
        vec![ReviewThread {
            comments: CommentConnection {
                nodes: vec![review_comment("a.rs"), review_comment("b.rs")],
                ..Default::default()
            },
            ..Default::default()
        }],
        vec![("a.rs".into(), 1), ("b.rs".into(), 1)]
    )]
    #[case(
        vec![ReviewThread {
            comments: CommentConnection {
                nodes: vec![
                    review_comment("a.rs"),
                    review_comment("a.rs"),
                    review_comment("b.rs"),
                ],
                ..Default::default()
            },
            ..Default::default()
        }],
        vec![("a.rs".into(), 2), ("b.rs".into(), 1)]
    )]
    fn summarize_files_counts_comments(
        #[case] threads: Vec<ReviewThread>,
        #[case] expected: Vec<(String, usize)>,
    ) {
        let summary = summarize_files(&threads);
        assert_eq!(summary, expected);
    }

    #[test]
    fn write_summary_outputs_text() {
        let summary = vec![("a.rs".into(), 2), ("b.rs".into(), 1)];
        let mut buf = Vec::new();
        write_summary(&mut buf, &summary).expect("write summary");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains("Summary:"));
        assert!(out.contains("a.rs: 2 comments"));
        assert!(out.contains("b.rs: 1 comment"));
    }

    #[test]
    fn write_summary_handles_empty() {
        let summary: Vec<(String, usize)> = Vec::new();
        let mut buf = Vec::new();
        write_summary(&mut buf, &summary).expect("write summary");
        assert!(buf.is_empty());
    }

    #[test]
    fn write_thread_emits_diff_once() {
        let diff = "@@ -1 +1 @@\n-old\n+new\n";
        let c1 = ReviewComment {
            diff_hunk: diff.into(),
            url: "http://u1".into(),
            ..Default::default()
        };
        let c2 = ReviewComment {
            diff_hunk: diff.into(),
            url: "http://u2".into(),
            ..Default::default()
        };
        let thread = ReviewThread {
            comments: CommentConnection {
                nodes: vec![c1, c2],
                ..Default::default()
            },
            ..Default::default()
        };
        let skin = MadSkin::default();
        let mut buf = Vec::new();
        write_thread(&mut buf, &skin, &thread).expect("write thread");
        let out = String::from_utf8(buf).expect("utf8");
        assert_eq!(out.matches("|-old").count(), 1);
        assert_eq!(out.matches("wrote:").count(), 2);
    }
}

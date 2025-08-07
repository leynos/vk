//! Entry point for the `vk` command line tool.
//!
//! `vk` fetches unresolved review comments from GitHub's GraphQL API,
//! summarizing them by file before printing each thread. When a thread has
//! multiple comments on the same diff, the diff is shown only once.
//! After all comments are printed, the tool displays an `end of code review`
//! banner so calling processes know the output has finished.
pub mod api;
mod cli_args;
mod diff;
mod graphql_queries;
mod html;
mod printer;
mod ref_parser;
mod reviews;
pub use crate::api::{GraphQLClient, paginate};
use crate::cli_args::{GlobalArgs, IssueArgs, PrArgs};
use crate::graphql_queries::{COMMENT_QUERY, ISSUE_QUERY, THREADS_QUERY};
use crate::printer::{print_reviews, write_thread};
use crate::ref_parser::{RepoInfo, parse_issue_reference, parse_pr_reference};
use crate::reviews::{fetch_reviews, latest_reviews};
use clap::{Parser, Subcommand};
use figment::error::{Error as FigmentError, Kind as FigmentKind};
use log::{error, warn};
use ortho_config::{OrthoConfig, OrthoError, load_and_merge_subcommand_for};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;
use std::sync::LazyLock;
use termimad::MadSkin;
use thiserror::Error;

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

#[derive(Error, Debug)]
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

static UTF8_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bUTF-?8\b").expect("valid regex"));

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
pub struct PageInfo {
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

async fn fetch_comment_page(
    client: &GraphQLClient,
    id: &str,
    cursor: Option<String>,
) -> Result<(Vec<ReviewComment>, PageInfo), VkError> {
    let wrapper: CommentNodeWrapper = client
        .run_query(COMMENT_QUERY, json!({ "id": id, "cursor": cursor.clone() }))
        .await?;
    let conn = wrapper
        .node
        .ok_or_else(|| {
            VkError::BadResponse(format!(
                "Missing comment node in response (id: {}, cursor: {})",
                id,
                cursor.as_deref().unwrap_or("None")
            ))
        })?
        .comments;
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

/// Create a [`GraphQLClient`], falling back to no transcript on failure.
///
/// This attempts to initialize the client with the provided `transcript`.
/// If the transcript cannot be created, it logs a warning and retries
/// without one.
#[expect(
    clippy::result_large_err,
    reason = "VkError has many variants but they are small"
)]
fn build_graphql_client(
    token: &str,
    transcript: Option<&std::path::PathBuf>,
) -> Result<GraphQLClient, VkError> {
    match GraphQLClient::new(token, transcript.cloned()) {
        Ok(c) => Ok(c),
        Err(e) => {
            warn!("failed to create transcript: {e}");
            GraphQLClient::new(token, None).map_err(Into::into)
        }
    }
}

#[allow(
    clippy::result_large_err,
    reason = "VkError has many variants but they are small"
)]
async fn run_pr(args: PrArgs, global: &GlobalArgs) -> Result<(), VkError> {
    let reference = args.reference.as_deref().ok_or(VkError::InvalidRef)?;
    let (repo, number) = parse_pr_reference(reference, global.repo.as_deref())?;
    let token = env::var("GITHUB_TOKEN").unwrap_or_default();
    if token.is_empty() {
        warn!("GITHUB_TOKEN not set, using anonymous API access");
    }
    if !locale_is_utf8() {
        warn!("terminal locale is not UTF-8; emojis may not render correctly");
    }

    let client = build_graphql_client(&token, global.transcript.as_ref())?;
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
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    if let Err(e) = print_reviews(&mut handle, &skin, &latest) {
        error!("error printing review: {e}");
    }

    for t in threads {
        if let Err(e) = print_thread(&skin, &t) {
            error!("error printing thread: {e}");
        }
    }
    print_end_banner();
    Ok(())
}

#[allow(
    clippy::result_large_err,
    reason = "VkError has many variants but they are small"
)]
async fn run_issue(args: IssueArgs, global: &GlobalArgs) -> Result<(), VkError> {
    let reference = args.reference.as_deref().ok_or(VkError::InvalidRef)?;
    let (repo, number) = parse_issue_reference(reference, global.repo.as_deref())?;
    let token = env::var("GITHUB_TOKEN").unwrap_or_default();
    if token.is_empty() {
        warn!("GITHUB_TOKEN not set, using anonymous API access");
    }
    if !locale_is_utf8() {
        warn!("terminal locale is not UTF-8; emojis may not render correctly");
    }

    let client = build_graphql_client(&token, global.transcript.as_ref())?;
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

#[allow(
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
    env_logger::init();
    let cli = Cli::parse();
    let mut global = GlobalArgs::load_from_iter(std::env::args_os().take(1))?;
    global.merge(cli.global);
    match cli.command {
        Commands::Pr(pr_cli) => {
            let args = load_with_reference_fallback::<PrArgs>(pr_cli.clone())?;
            run_pr(args, &global).await
        }
        Commands::Issue(issue_cli) => {
            let args = load_with_reference_fallback::<IssueArgs>(issue_cli.clone())?;
            run_issue(args, &global).await
        }
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
    use crate::printer::{write_comment_body, write_review, write_thread};
    use crate::reviews::PullRequestReview;
    use chrono::Utc;
    use rstest::*;

    fn set_var<K: AsRef<std::ffi::OsStr>, V: AsRef<std::ffi::OsStr>>(key: K, value: V) {
        // SAFETY: manipulating environment variables in tests is safe because tests run serially.
        unsafe { std::env::set_var(key, value) }
    }

    fn remove_var<K: AsRef<std::ffi::OsStr>>(key: K) {
        // SAFETY: manipulating environment variables in tests is safe because tests run serially.
        unsafe { std::env::remove_var(key) }
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

    #[test]
    fn comment_body_collapses_details() {
        let comment = ReviewComment {
            body: "<details><summary>note</summary>hidden</details>".into(),
            ..Default::default()
        };
        let skin = MadSkin::default();
        let mut buf = Vec::new();
        write_comment_body(&mut buf, &skin, &comment).expect("write comment");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains("\u{25B6} note"));
        assert!(!out.contains("hidden"));
    }

    #[test]
    fn review_body_collapses_details() {
        let review = PullRequestReview {
            body: "<details><summary>hello</summary>bye</details>".into(),
            submitted_at: Utc::now(),
            state: "APPROVED".into(),
            author: None,
        };
        let skin = MadSkin::default();
        let mut buf = Vec::new();
        write_review(&mut buf, &skin, &review).expect("write review");
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains("\u{25B6} hello"));
        assert!(!out.contains("bye"));
    }

    #[tokio::test]
    async fn run_query_missing_nodes_reports_path() {
        use third_wheel::hyper::{
            Body, Request, Response, Server, StatusCode,
            service::{make_service_fn, service_fn},
        };

        let body = serde_json::json!({
            "data": {
                "repository": {
                    "pullRequest": {
                        "reviewThreads": {
                            "pageInfo": { "hasNextPage": false, "endCursor": null }
                        }
                    }
                }
            }
        })
        .to_string();

        let make_svc = make_service_fn(move |_conn| {
            let body = body.clone();
            async move {
                Ok::<_, std::convert::Infallible>(service_fn(move |_req: Request<Body>| {
                    let body = body.clone();
                    async move {
                        Ok::<_, std::convert::Infallible>(
                            Response::builder()
                                .status(StatusCode::OK)
                                .header("Content-Type", "application/json")
                                .body(Body::from(body.clone()))
                                .expect("failed to build HTTP response"),
                        )
                    }
                }))
            }
        });

        let server = Server::bind(
            &"127.0.0.1:0"
                .parse()
                .expect("failed to parse server address"),
        )
        .serve(make_svc);
        let addr = server.local_addr();
        let join = tokio::spawn(server);

        let client = GraphQLClient::with_endpoint("token", &format!("http://{addr}"), None)
            .expect("failed to create GraphQL client");

        let repo = RepoInfo {
            owner: "o".into(),
            name: "r".into(),
        };
        let result = fetch_review_threads(&client, &repo, 1).await;
        let err = result.expect_err("expected error");
        let err_msg = format!("{err}");
        assert!(
            err_msg.contains("repository.pullRequest.reviewThreads"),
            "Error should contain full JSON path"
        );
        assert!(
            err_msg.contains("snippet:"),
            "Error should contain JSON snippet"
        );

        join.abort();
        let _ = join.await;
    }
}

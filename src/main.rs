//! Entry point for the `vk` command line tool.
//!
//! This module provides the main entry point and orchestrates the `vk` command
//! line tool, which fetches unresolved review comments from GitHub's GraphQL
//! API. Passing a pull request reference with a `#discussion_r<ID>` fragment
//! prints only the matching thread starting from that comment. The unresolved
//! filter remains in effect and any file filters are ignored. The core
//! functionality is delegated to specialised modules:
//! `review_threads` for fetching review data, `issues` for issue retrieval,
//! `summary` for summarizing comments. Configuration defaults are merged using
//! `ortho_config`. When a thread has multiple comments on the same diff, the diff
//! is shown only once. Output is framed by a `code review` banner at the start
//! and an `end of code review` banner at the end so calling processes can
//! reliably detect boundaries. A `review comments` banner separates reviewer
//! summaries from the comment threads. When no threads are present this
//! banner is omitted. Banner helpers [`print_start_banner`],
//! [`print_comments_banner`] and [`print_end_banner`] frame output while
//! summary utilities [`print_summary`], [`summarize_files`], and
//! [`write_summary`] collate comments so consumers can reuse the framing
//! logic.

pub mod api;
mod boxed;
mod cli_args;
// configuration helpers have been folded into `ortho_config`
mod diff;
mod graphql_queries;
mod html;
mod issues;
mod printer;
mod ref_parser;
mod resolve;
mod review_threads;
mod reviews;
mod summary;
#[cfg(test)]
mod test_utils;

pub use crate::api::{GraphQLClient, paginate};
pub use issues::{Issue, fetch_issue};
use review_threads::thread_for_comment;
pub use review_threads::{
    CommentConnection, PageInfo, ReviewComment, ReviewThread, User, fetch_review_threads,
    fetch_review_threads_with_resolution, filter_threads_by_files,
};
use summary::{
    print_comments_banner, print_end_banner, print_start_banner, print_summary, summarize_files,
};

use crate::cli_args::{GlobalArgs, IssueArgs, PrArgs, ResolveArgs};
use crate::printer::{print_reviews, write_thread};
use crate::ref_parser::{RepoInfo, parse_issue_reference, parse_pr_thread_reference};
use crate::reviews::{PullRequestReview, fetch_reviews, latest_reviews};
use clap::{Parser, Subcommand};
use log::{error, warn};
use ortho_config::{OrthoConfig, subcommand::load_and_merge_subcommand_for};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::env;
use std::io::{ErrorKind, Write};
use std::sync::LazyLock;
use termimad::MadSkin;
use thiserror::Error;

struct PrContext {
    repo: RepoInfo,
    number: u64,
    comment_id: Option<u64>,
    client: GraphQLClient,
}

#[derive(Subcommand, Deserialize, Serialize, Clone, Debug)]
enum Commands {
    /// Show unresolved pull request comments
    ///
    /// Passing a `#discussion_r<ID>` fragment prints only that discussion
    /// thread starting from the referenced comment. When a fragment is
    /// provided, both resolved and unresolved threads are searched.
    /// Without a fragment, only unresolved threads are shown.
    Pr(PrArgs),
    /// Read a GitHub issue (todo)
    Issue(IssueArgs),
    /// Resolve a pull request comment
    Resolve(ResolveArgs),
}

#[derive(Debug, Parser)]
#[command(
    name = "vk",
    about = "View Komments - show unresolved PR comments",
    version,
    subcommand_required = true,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: crate::Commands,
    #[command(flatten)]
    global: GlobalArgs,
}

/// Error type for the `vk` binary.
///
/// String payloads and external errors are boxed to keep the enum small. A
/// `Cow<'static, str>` would avoid allocations for static strings but would
/// enlarge the type and still allocate for dynamic values, so boxing is
/// preferred.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum VkError {
    #[error("unable to determine repository")]
    RepoNotFound,
    #[error("request failed: {0}")]
    Request(#[from] Box<reqwest::Error>),
    #[error("request failed when running {context}: {source}")]
    RequestContext {
        context: Box<str>,
        #[source]
        source: Box<reqwest::Error>,
    },
    #[error("invalid reference")]
    InvalidRef,
    #[error("pull request number out of range")]
    InvalidNumber,
    #[error("expected URL path segment in {expected:?}, found '{found}'")]
    WrongResourceType {
        expected: &'static [&'static str],
        found: Box<str>,
    },
    #[error("missing comment path at index {index} in thread {thread_id}")]
    EmptyCommentPath { thread_id: Box<str>, index: usize },
    #[error("bad response: {0}")]
    BadResponse(Box<str>),
    #[error("empty GraphQL response (status {status}) for {operation}: {snippet}")]
    EmptyResponse {
        status: u16,
        operation: Box<str>,
        snippet: Box<str>,
    },
    #[error("malformed response (status {status}): {message} | snippet:{snippet}")]
    BadResponseSerde {
        status: u16,
        message: Box<str>,
        snippet: Box<str>,
    },
    #[error("API errors: {0}")]
    ApiErrors(Box<str>),
    #[error("io error: {0}")]
    Io(#[from] Box<std::io::Error>),
    #[error("configuration error: {0}")]
    Config(#[from] Box<ortho_config::OrthoError>),
}

/// Implement `From<$source>` for `VkError` by boxing the source into `$variant`.
macro_rules! boxed_error_from {
    ($source:ty, $variant:ident) => {
        impl From<$source> for VkError {
            fn from(source: $source) -> Self {
                Self::$variant(Box::new(source))
            }
        }
    };
}

boxed_error_from!(reqwest::Error, Request);
boxed_error_from!(std::io::Error, Io);
boxed_error_from!(ortho_config::OrthoError, Config);

static UTF8_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bUTF-?8\b").expect("valid regex"));

/// Print a review thread to stdout.
///
/// This simply calls [`write_thread`] with a locked `stdout` handle.
fn print_thread(skin: &MadSkin, thread: &ReviewThread) -> anyhow::Result<()> {
    write_thread(std::io::stdout().lock(), skin, thread)
}

/// Create a [`GraphQLClient`], falling back to no transcript on failure.
///
/// This attempts to initialize the client with the provided `transcript`.
/// If the transcript cannot be created, it logs a warning and retries
/// without one.
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

fn caused_by_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|c| {
        c.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == ErrorKind::BrokenPipe)
    })
}

fn is_broken_pipe_io(err: &std::io::Error) -> bool {
    err.kind() == ErrorKind::BrokenPipe
}

fn handle_banner<F>(print: F, label: &str) -> bool
where
    F: FnOnce() -> std::io::Result<()>,
{
    if let Err(e) = print() {
        if is_broken_pipe_io(&e) {
            return true;
        }
        error!("error printing {label} banner: {e}");
    }
    false
}

/// Prepare PR context, validate environment and print the start banner.
///
/// Returns `Ok(None)` when standard output is closed before printing.
fn setup_pr_output(args: &PrArgs, global: &GlobalArgs) -> Result<Option<PrContext>, VkError> {
    let reference = args.reference.as_deref().ok_or(VkError::InvalidRef)?;
    let (repo, number, comment) = parse_pr_thread_reference(reference, global.repo.as_deref())?;
    let token = env::var("GITHUB_TOKEN").unwrap_or_default();
    if token.is_empty() {
        warn!("GITHUB_TOKEN not set, using anonymous API access");
    }
    if !locale_is_utf8() {
        warn!("terminal locale is not UTF-8; emojis may not render correctly");
    }
    if handle_banner(print_start_banner, "start") {
        return Ok(None);
    }
    let client = build_graphql_client(&token, global.transcript.as_ref())?;
    Ok(Some(PrContext {
        repo,
        number,
        comment_id: comment,
        client,
    }))
}

/// Print an appropriate message when no threads match and append the end banner.
#[expect(
    clippy::unnecessary_wraps,
    reason = "returns Result for interface symmetry"
)]
fn handle_empty_threads(files: &[String], comment: Option<u64>) -> Result<(), VkError> {
    let msg = match (comment.is_some(), files.is_empty()) {
        (true, _) => "No unresolved comments in the requested discussion.",
        (false, true) => "No unresolved comments.",
        (false, false) => "No unresolved comments for the specified files.",
    };
    if let Err(e) = writeln!(std::io::stdout().lock(), "{msg}") {
        if is_broken_pipe_io(&e) {
            return Ok(());
        }
        error!("error writing empty state: {e}");
    }
    if handle_banner(print_end_banner, "end") {
        return Ok(());
    }
    Ok(())
}

/// Render the summary, reviews and threads, then print the closing banner.
#[expect(clippy::unnecessary_wraps, reason = "future error cases may emerge")]
fn generate_pr_output(
    threads: Vec<ReviewThread>,
    reviews: Vec<PullRequestReview>,
) -> Result<(), VkError> {
    let summary = summarize_files(&threads);
    print_summary(&summary);

    let skin = MadSkin::default();
    let latest = latest_reviews(reviews);
    {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        if let Err(e) = print_reviews(&mut handle, &skin, &latest) {
            if caused_by_broken_pipe(&e) {
                return Ok(());
            }
            error!("error printing review: {e}");
        }
    } // drop handle before locking stdout again

    // Stop if the comments banner cannot be written, usually indicating stdout
    // has been closed, as printing threads would also fail.
    if handle_banner(print_comments_banner, "comments") {
        return Ok(());
    }

    for t in threads {
        if let Err(e) = print_thread(&skin, &t) {
            if caused_by_broken_pipe(&e) {
                return Ok(());
            }
            error!("error printing thread: {e}");
        }
    }

    if handle_banner(print_end_banner, "end") {
        return Ok(());
    }
    Ok(())
}

async fn run_pr(args: PrArgs, global: &GlobalArgs) -> Result<(), VkError> {
    let Some(PrContext {
        repo,
        number,
        comment_id: comment,
        client,
    }) = setup_pr_output(&args, global)?
    else {
        return Ok(());
    };

    let threads = {
        // When a discussion fragment is given, fetch ALL threads (resolved + unresolved)
        // and filter to the specific thread. Otherwise, fetch only unresolved threads
        // and apply file filters.
        let include_resolved = comment.is_some();
        let all =
            fetch_review_threads_with_resolution(&client, &repo, number, include_resolved).await?;

        if let Some(comment_id) = comment {
            thread_for_comment(all, comment_id).into_iter().collect()
        } else {
            filter_threads_by_files(all, &args.files)
        }
    };

    if threads.is_empty() {
        handle_empty_threads(&args.files, comment)?;
        return Ok(());
    }

    let reviews = fetch_reviews(&client, &repo, number).await?;
    generate_pr_output(threads, reviews)
}

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

async fn run_resolve(args: ResolveArgs, global: &GlobalArgs) -> Result<(), VkError> {
    let (repo, number, comment) =
        parse_pr_thread_reference(&args.reference, global.repo.as_deref())?;
    let comment_id = comment.ok_or(VkError::InvalidRef)?;
    let token = env::var("GITHUB_TOKEN").unwrap_or_default();
    if token.is_empty() {
        warn!("GITHUB_TOKEN not set, using anonymous API access");
    }
    resolve::resolve_comment(&token, &repo, number, comment_id, args.message).await
}

#[tokio::main]
async fn main() -> Result<(), VkError> {
    env_logger::init();
    let cli = Cli::parse();
    let mut global = GlobalArgs::load_from_iter(std::env::args_os().take(1))?;
    global.merge(cli.global);
    match cli.command {
        Commands::Pr(pr_cli) => {
            let args = load_and_merge_subcommand_for(&pr_cli)?;
            run_pr(args, &global).await
        }
        Commands::Issue(issue_cli) => {
            let args = load_and_merge_subcommand_for(&issue_cli)?;
            run_issue(args, &global).await
        }
        Commands::Resolve(resolve_cli) => {
            let args = load_and_merge_subcommand_for(&resolve_cli)?;
            run_resolve(args, &global).await
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
    use crate::test_utils::{remove_var, set_var};
    use chrono::Utc;
    use serial_test::serial;

    #[test]
    fn cli_loads_repo_from_flag() {
        let cli = Cli::try_parse_from(["vk", "--repo", "foo/bar", "pr", "1"]).expect("parse cli");
        assert_eq!(cli.global.repo.as_deref(), Some("foo/bar"));
    }

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
            Commands::Pr(args) => {
                assert_eq!(args.reference.as_deref(), Some("123"));
                assert!(args.files.is_empty());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn pr_subcommand_parses_files() {
        let cli =
            Cli::try_parse_from(["vk", "pr", "123", "src/lib.rs", "README.md"]).expect("parse cli");
        match cli.command {
            Commands::Pr(args) => {
                assert_eq!(args.reference.as_deref(), Some("123"));
                assert_eq!(args.files, ["src/lib.rs", "README.md"]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn issue_subcommand_parses() {
        let cli = Cli::try_parse_from(["vk", "issue", "123"]).expect("parse cli");
        match cli.command {
            Commands::Issue(args) => assert_eq!(args.reference.as_deref(), Some("123")),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn resolve_subcommand_parses() {
        let cli = Cli::try_parse_from(["vk", "resolve", "83#discussion_r1"]).expect("parse cli");
        match cli.command {
            Commands::Resolve(args) => {
                assert_eq!(args.reference, "83#discussion_r1");
                assert!(args.message.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn resolve_subcommand_parses_message() {
        let cli = Cli::try_parse_from(["vk", "resolve", "83#discussion_r1", "-m", "done"])
            .expect("parse cli");
        match cli.command {
            Commands::Resolve(args) => {
                assert_eq!(args.reference, "83#discussion_r1");
                assert_eq!(args.message.as_deref(), Some("done"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn version_flag_displays_version() {
        let err = Cli::try_parse_from(["vk", "--version"]).expect_err("display version");
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
        assert!(err.to_string().contains(env!("CARGO_PKG_VERSION")));
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

    #[test]
    fn handle_banner_returns_true_on_broken_pipe() {
        let broken_pipe =
            || -> std::io::Result<()> { Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe)) };
        assert!(super::handle_banner(broken_pipe, "start"));
    }

    #[test]
    fn handle_banner_logs_and_returns_false_on_other_errors() {
        let other_err = || -> std::io::Result<()> { Err(std::io::Error::other("boom")) };
        assert!(!super::handle_banner(other_err, "end"));
    }
}

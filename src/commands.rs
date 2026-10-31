//! Command execution helpers for `vk`.
//!
//! This module owns the runtime flow for each subcommand, including token
//! resolution, API client setup, and rendering output to the terminal.

use crate::auth::resolve_github_token;
use crate::branch_pr::fetch_pr_for_branch;
use crate::cli_args::{GlobalArgs, IssueArgs, PrArgs, ResolveArgs};
use crate::environment;
use crate::ref_parser::{
    RepoInfo, is_fragment_only, parse_fragment_only, parse_issue_reference,
    parse_pr_thread_reference,
};
use crate::review_threads::thread_for_comment;
use crate::reviews::{PullRequestReview, fetch_reviews};
use crate::summary::{
    print_comments_banner, print_end_banner, print_start_banner, print_summary, summarize_files,
};
use crate::{
    FetchOptions, GraphQLClient, ReviewThread, VkError, fetch_issue,
    fetch_review_threads_with_options, filter_threads_by_files, resolve,
};
use branch::resolve_branch_and_repo;
use output::{
    handle_banner, handle_print_result, is_broken_pipe_kind, print_reviews_block,
    print_threads_block,
};
use std::io::Write;
use termimad::MadSkin;
use tracing::{error, warn};

#[cfg(feature = "unstable-rest-resolve")]
use std::time::Duration;

/// Default total HTTP timeout for REST comment resolution, in seconds.
#[cfg(feature = "unstable-rest-resolve")]
const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 10;
/// Default connection timeout for REST comment resolution, in seconds.
#[cfg(feature = "unstable-rest-resolve")]
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 5;

/// Context shared by the pull-request output and fetching stages.
struct PrContext {
    /// Repository containing the pull request.
    repo: RepoInfo,
    /// Pull request number.
    number: u64,
    /// Optional discussion comment used to narrow the output.
    comment_id: Option<u64>,
    /// GraphQL client used to fetch pull-request data.
    client: GraphQLClient,
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
            GraphQLClient::new(token, None)
        }
    }
}

/// Warn when authentication or a UTF-8 terminal locale is unavailable.
fn warn_on_missing_token_and_locale(token: &str) {
    if token.is_empty() {
        warn!("GitHub token not set, using anonymous API access");
    }
    if !locale_is_utf8() {
        warn!("terminal locale is not UTF-8; emojis may not render correctly");
    }
}

/// Resolve the PR reference, detecting from branch when necessary.
///
/// Handles three cases:
/// 1. No reference: detect PR from current branch
/// 2. Fragment only (`#discussion_r<ID>`): detect PR from branch, extract comment ID
/// 3. Full reference: use existing parsing
///
/// When auto-detecting from branch, uses the origin remote's repository owner
/// to disambiguate between multiple forks with the same branch name.
async fn resolve_pr_reference(
    reference: Option<&str>,
    default_repo: Option<&str>,
    client: &GraphQLClient,
) -> Result<(RepoInfo, u64, Option<u64>), VkError> {
    match reference {
        None => {
            let ctx = resolve_branch_and_repo(default_repo)?;
            let number =
                fetch_pr_for_branch(client, &ctx.repo, &ctx.branch, ctx.head_owner.as_deref())
                    .await?;
            Ok((ctx.repo, number, None))
        }
        Some(input) if is_fragment_only(input) => {
            let comment_id = parse_fragment_only(input)?;
            let ctx = resolve_branch_and_repo(default_repo)?;
            let number =
                fetch_pr_for_branch(client, &ctx.repo, &ctx.branch, ctx.head_owner.as_deref())
                    .await?;
            Ok((ctx.repo, number, Some(comment_id)))
        }
        Some(input) => parse_pr_thread_reference(input, default_repo),
    }
}

/// Prepare PR context, validate environment and print the start banner.
///
/// Returns `Ok(None)` when standard output is closed before printing.
async fn setup_pr_output(
    args: &PrArgs,
    global: &GlobalArgs,
    cli_token: Option<&str>,
) -> Result<Option<PrContext>, VkError> {
    let token = resolve_github_token(cli_token, global.github_token.as_deref());
    warn_on_missing_token_and_locale(&token);
    if handle_banner(print_start_banner, "start") {
        return Ok(None);
    }
    let client = build_graphql_client(&token, global.transcript.as_ref())?;
    let (repo, number, comment) =
        resolve_pr_reference(args.reference.as_deref(), global.repo.as_deref(), &client).await?;
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
        if is_broken_pipe_kind(e.kind()) {
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
fn generate_pr_output(threads: Vec<ReviewThread>, reviews: Vec<PullRequestReview>) {
    let summary = summarize_files(&threads);
    print_summary(&summary);

    let skin = MadSkin::default();
    if print_reviews_block(&skin, reviews) {
        return;
    }

    // Stop if the comments banner cannot be written, usually indicating stdout
    // has been closed, as printing threads would also fail.
    if handle_banner(print_comments_banner, "comments") {
        return;
    }

    if print_threads_block(&skin, threads) {
        return;
    }

    let _ = handle_banner(print_end_banner, "end");
}

/// Run the `pr` command and print pull request review threads.
///
/// Parameters:
/// - `args`: CLI arguments controlling PR behaviour and filtering.
/// - `global`: global CLI options, including repository defaults and transcript
///   output.
/// - `cli_token`: optional authentication token from the CLI.
///
/// Returns `Ok(())` on success or a `VkError` when the reference is invalid or
/// the API request fails. When a discussion comment ID is present, resolved
/// threads are included; otherwise unresolved threads are filtered by file.
pub async fn run_pr(
    args: PrArgs,
    global: &GlobalArgs,
    cli_token: Option<&str>,
) -> Result<(), VkError> {
    let Some(PrContext {
        repo,
        number,
        comment_id: comment,
        client,
    }) = setup_pr_output(&args, global, cli_token).await?
    else {
        return Ok(());
    };

    // When a discussion fragment is given, fetch ALL threads (resolved + unresolved)
    // and filter to the specific thread. Otherwise, fetch only unresolved threads
    // and apply file filters.
    let include_resolved = comment.is_some();
    let threads = fetch_review_threads_with_options(
        &client,
        &repo,
        number,
        FetchOptions {
            include_resolved,
            include_outdated: args.show_outdated,
        },
    )
    .await
    .map(|threads| {
        if let Some(comment_id) = comment {
            thread_for_comment(threads, comment_id)
                .into_iter()
                .collect()
        } else {
            filter_threads_by_files(threads, &args.files)
        }
    })?;

    if threads.is_empty() {
        handle_empty_threads(&args.files, comment)?;
        return Ok(());
    }

    let reviews = fetch_reviews(&client, &repo, number).await?;
    generate_pr_output(threads, reviews);
    Ok(())
}

/// Run the `issue` command and print issue details.
///
/// Parameters:
/// - `args`: CLI arguments for selecting an issue reference.
/// - `global`: global CLI options, including repository defaults and transcript
///   output.
/// - `cli_token`: optional authentication token from the CLI.
///
/// Returns `Ok(())` on success or a `VkError` when the reference is invalid or
/// the API request fails.
pub async fn run_issue(
    args: IssueArgs,
    global: &GlobalArgs,
    cli_token: Option<&str>,
) -> Result<(), VkError> {
    let reference = args.reference.as_deref().ok_or(VkError::InvalidRef)?;
    let (repo, number) = parse_issue_reference(reference, global.repo.as_deref())?;
    let token = resolve_github_token(cli_token, global.github_token.as_deref());
    warn_on_missing_token_and_locale(&token);

    let client = build_graphql_client(&token, global.transcript.as_ref())?;
    let issue = fetch_issue(&client, &repo, number).await?;

    let skin = MadSkin::default();
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    if handle_print_result(
        skin.write_text_on(&mut handle, &format!("**{}**", issue.title)),
        "issue title",
    ) {
        return Ok(());
    }
    if handle_print_result(skin.write_text_on(&mut handle, &issue.body), "issue body") {
        return Ok(());
    }
    if handle_print_result(writeln!(handle), "issue footer") {
        return Ok(());
    }
    Ok(())
}

/// Run the `resolve` command to resolve a pull request comment.
///
/// Parameters:
/// - `args`: CLI arguments containing the discussion reference and message.
/// - `global`: global CLI options, including repository defaults and timeouts.
/// - `cli_token`: optional authentication token from the CLI.
///
/// Returns `Ok(())` on success or a `VkError` when the reference is invalid or
/// authentication is missing. The message is used only when the
/// `unstable-rest-resolve` feature is enabled, and HTTP timeout settings are
/// applied to that flow.
pub async fn run_resolve(
    args: ResolveArgs,
    global: &GlobalArgs,
    cli_token: Option<&str>,
) -> Result<(), VkError> {
    let (repo, number, comment) =
        parse_pr_thread_reference(&args.reference, global.repo.as_deref())?;
    let comment_id = comment.ok_or(VkError::InvalidRef)?;
    let token = resolve_github_token(cli_token, global.github_token.as_deref());
    if token.is_empty() {
        return Err(VkError::MissingAuth);
    }
    #[cfg(feature = "unstable-rest-resolve")]
    {
        let http_timeout =
            Duration::from_secs(global.http_timeout.unwrap_or(DEFAULT_HTTP_TIMEOUT_SECS));
        let connect_timeout = Duration::from_secs(
            global
                .connect_timeout
                .unwrap_or(DEFAULT_CONNECT_TIMEOUT_SECS),
        );
        resolve::resolve_comment(
            &token,
            resolve::CommentRef {
                repo: &repo,
                pull_number: number,
                comment_id,
            },
            args.message,
            http_timeout,
            connect_timeout,
        )
        .await
    }
    #[cfg(not(feature = "unstable-rest-resolve"))]
    {
        #[expect(
            unused_variables,
            reason = "message is only used with unstable-rest-resolve enabled"
        )]
        let message = args.message;
        resolve::resolve_comment(
            &token,
            resolve::CommentRef {
                repo: &repo,
                pull_number: number,
                comment_id,
            },
        )
        .await
    }
}

/// Return whether the first configured locale variable is UTF-8.
fn locale_is_utf8() -> bool {
    for key in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(value) = environment::var(key) {
            return crate::UTF8_RE
                .as_ref()
                .is_some_and(|re| re.is_match(&value));
        }
    }
    false
}

mod branch;
mod output;
#[cfg(test)]
mod tests;

//! Command execution helpers for `vk`.
//!
//! This module owns the runtime flow for each subcommand, including token
//! resolution, API client setup, and rendering output to the terminal.

use crate::auth::resolve_github_token;
use crate::cli_args::{GlobalArgs, IssueArgs, PrArgs, ResolveArgs};
use crate::printer::{print_reviews, write_thread};
use crate::ref_parser::{RepoInfo, parse_issue_reference, parse_pr_thread_reference};
use crate::review_threads::thread_for_comment;
use crate::reviews::{PullRequestReview, fetch_reviews, latest_reviews};
use crate::summary::{
    print_comments_banner, print_end_banner, print_start_banner, print_summary, summarize_files,
};
use crate::{
    FetchOptions, GraphQLClient, ReviewThread, VkError, fetch_issue,
    fetch_review_threads_with_options, filter_threads_by_files, resolve,
};
use std::io::{ErrorKind, Write};
use termimad::MadSkin;
use tracing::{error, warn};
use vk::environment;

#[cfg(feature = "unstable-rest-resolve")]
use std::time::Duration;

pub struct PrContext {
    repo: RepoInfo,
    number: u64,
    comment_id: Option<u64>,
    client: GraphQLClient,
}

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
    let token = resolve_github_token(global);
    if token.is_empty() {
        warn!("GitHub token not set, using anonymous API access");
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

pub async fn run_pr(args: PrArgs, global: &GlobalArgs) -> Result<(), VkError> {
    let Some(PrContext {
        repo,
        number,
        comment_id: comment,
        client,
    }) = setup_pr_output(&args, global)?
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
    generate_pr_output(threads, reviews)
}

pub async fn run_issue(args: IssueArgs, global: &GlobalArgs) -> Result<(), VkError> {
    let reference = args.reference.as_deref().ok_or(VkError::InvalidRef)?;
    let (repo, number) = parse_issue_reference(reference, global.repo.as_deref())?;
    let token = resolve_github_token(global);
    if token.is_empty() {
        warn!("GitHub token not set, using anonymous API access");
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

pub async fn run_resolve(args: ResolveArgs, global: &GlobalArgs) -> Result<(), VkError> {
    let (repo, number, comment) =
        parse_pr_thread_reference(&args.reference, global.repo.as_deref())?;
    let comment_id = comment.ok_or(VkError::InvalidRef)?;
    let token = resolve_github_token(global);
    if token.is_empty() {
        return Err(VkError::MissingAuth);
    }
    #[cfg(feature = "unstable-rest-resolve")]
    {
        let http_timeout = Duration::from_secs(global.http_timeout.unwrap_or(10));
        let connect_timeout = Duration::from_secs(global.connect_timeout.unwrap_or(5));
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
        let _ = args.message;
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

fn locale_is_utf8() -> bool {
    environment::var("LC_ALL")
        .or_else(|_| environment::var("LC_CTYPE"))
        .or_else(|_| environment::var("LANG"))
        .map(|v| crate::UTF8_RE.is_match(&v))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::handle_banner;
    use super::locale_is_utf8;
    use crate::test_utils::{remove_var, set_var};
    use serial_test::serial;
    use vk::environment;

    #[test]
    #[serial]
    fn detect_utf8_locale() {
        let old_all = environment::var("LC_ALL").ok();
        let old_ctype = environment::var("LC_CTYPE").ok();
        let old_lang = environment::var("LANG").ok();

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
    fn handle_banner_returns_true_on_broken_pipe() {
        let broken_pipe =
            || -> std::io::Result<()> { Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe)) };
        assert!(handle_banner(broken_pipe, "start"));
    }

    #[test]
    fn handle_banner_logs_and_returns_false_on_other_errors() {
        let other_err = || -> std::io::Result<()> { Err(std::io::Error::other("boom")) };
        assert!(!handle_banner(other_err, "end"));
    }
}

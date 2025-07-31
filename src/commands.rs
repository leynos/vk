//! High level command handlers.

use crate::api::{VkError, build_graphql_client, fetch_issue, fetch_review_threads};
use crate::cli_args::{GlobalArgs, IssueArgs, PrArgs};
use crate::printer::{print_end_banner, print_summary, print_thread, summarize_files};
use crate::references::{parse_issue_reference, parse_pr_reference};
use crate::reviews::{fetch_reviews, latest_reviews, print_reviews};
use figment::error::{Error as FigmentError, Kind as FigmentKind};
use log::{error, warn};
use ortho_config::{OrthoConfig, OrthoError, load_and_merge_subcommand_for};
use std::env;
use termimad::MadSkin;

/// Fetch and display unresolved comments for a pull request.
///
/// # Errors
///
/// Returns `VkError` if any API call fails.
pub async fn run_pr(args: PrArgs, global: &GlobalArgs) -> Result<(), VkError> {
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
    print_reviews(&skin, &latest);
    for t in threads {
        if let Err(e) = print_thread(&skin, &t) {
            error!("error printing thread: {e}");
        }
    }
    print_end_banner();
    Ok(())
}

/// Detect whether the current locale uses UTF-8 encoding.
#[must_use]
pub fn locale_is_utf8() -> bool {
    use std::sync::LazyLock;
    static UTF8_REGEX: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new("(?i)\\bUTF-?8\\b").expect("valid regex"));
    env::var("LC_ALL")
        .or_else(|_| env::var("LC_CTYPE"))
        .or_else(|_| env::var("LANG"))
        .map(|v| UTF8_REGEX.is_match(&v))
        .unwrap_or(false)
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
/// Load configuration, falling back to CLI arguments when `reference` is missing.
///
/// # Errors
///
/// Returns `OrthoError` if configuration loading fails for any reason other
/// than a missing `reference` field.
pub fn load_with_reference_fallback<T>(cli_args: T) -> Result<T, OrthoError>
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
/// Display the contents of a GitHub issue.
///
/// # Errors
///
/// Returns `VkError` if fetching the issue fails.
pub async fn run_issue(args: IssueArgs, global: &GlobalArgs) -> Result<(), VkError> {
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
    println!("[1m{}[0m", issue.title);
    skin.print_text(&issue.body);
    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// # Safety
    ///
    /// Modifies global environment variables. Only safe when tests are run
    /// serially to avoid race conditions.
    fn set_var<K: AsRef<std::ffi::OsStr>, V: AsRef<std::ffi::OsStr>>(key: K, value: V) {
        unsafe { std::env::set_var(key, value) }
    }

    /// # Safety
    ///
    /// Modifies global environment variables. Only safe when tests are run
    /// serially to avoid race conditions.
    fn remove_var<K: AsRef<std::ffi::OsStr>>(key: K) {
        unsafe { std::env::remove_var(key) }
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
}

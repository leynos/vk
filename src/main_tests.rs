//! Tests for the `vk` binary entry point and top-level rendering behaviour.

use super::*;
use crate::printer::{write_comment_body, write_review, write_thread};
use crate::reviews::PullRequestReview;
use crate::test_utils::{EnvGuard, invalid_http_timeout_guard};
use chrono::Utc;
use ortho_config::OrthoConfig;
use rstest::{fixture, rstest};
use serial_test::serial;
use std::ffi::OsString;
use std::sync::Arc;
use termimad::MadSkin;

/// Parse CLI arguments and extract `PrArgs` from the `Pr` subcommand.
///
/// # Panics
///
/// Panics if parsing fails or if the command is not `Commands::Pr`.
fn parse_pr_args(args: &[&str]) -> PrArgs {
    let cli = Cli::try_parse_from(args).expect("parse cli");
    match cli.command {
        Commands::Pr(pr_args) => pr_args,
        _ => panic!("expected Pr command, got different variant"),
    }
}

/// Parse CLI arguments and extract `ResolveArgs` from the `Resolve`
/// subcommand.
///
/// # Panics
///
/// Panics if parsing fails or if the command is not `Commands::Resolve`.
fn parse_resolve_args(args: &[&str]) -> ResolveArgs {
    let cli = Cli::try_parse_from(args).expect("parse cli");
    match cli.command {
        Commands::Resolve(resolve_args) => resolve_args,
        _ => panic!("expected Resolve command, got different variant"),
    }
}

#[rstest(
    flag,
    value,
    expected_repo,
    expected_github_token,
    case("--repo", "foo/bar", Some("foo/bar"), None),
    case("--github-token", "token", None, Some("token"))
)]
fn cli_loads_global_flags(
    flag: &str,
    value: &str,
    expected_repo: Option<&str>,
    expected_github_token: Option<&str>,
) {
    let cli = Cli::try_parse_from(["vk", flag, value, "pr", "1"]).expect("parse cli");
    assert_eq!(cli.global.repo.as_deref(), expected_repo);
    assert_eq!(cli.global.github_token.as_deref(), expected_github_token);
}

fn assert_is_send_sync<T: Send + Sync>() {}

#[fixture]
fn invalid_http_timeout() -> EnvGuard {
    invalid_http_timeout_guard()
}

/// Holds the invalid-timeout guard alongside the resulting configuration error.
struct InvalidTimeoutError {
    _guard: EnvGuard,
    error: Arc<ortho_config::OrthoError>,
}

/// Load `GlobalArgs` while the invalid-timeout fixture keeps
/// `VK_HTTP_TIMEOUT` invalid.
///
/// # Panics
///
/// Panics if `GlobalArgs::load_from_iter` unexpectedly succeeds.
#[fixture]
fn invalid_http_timeout_error(invalid_http_timeout: EnvGuard) -> InvalidTimeoutError {
    let error = GlobalArgs::load_from_iter(std::iter::once(OsString::from("vk")))
        .expect_err("invalid VK_HTTP_TIMEOUT should fail");
    InvalidTimeoutError {
        _guard: invalid_http_timeout,
        error,
    }
}

#[test]
fn vk_error_is_send_and_sync() {
    assert_is_send_sync::<VkError>();
}

#[rstest]
#[serial]
fn vk_error_config_from_arc_preserves_allocation(invalid_http_timeout_error: InvalidTimeoutError) {
    let err = invalid_http_timeout_error.error;
    let original = err.clone();
    let converted: VkError = err.into();
    match converted {
        VkError::Config(stored) => assert!(
            Arc::ptr_eq(&stored, &original),
            "conversion from Arc should preserve the original allocation"
        ),
        other => panic!("expected VkError::Config, got {other:?}"),
    }
}

#[rstest]
#[serial]
fn vk_error_config_from_owned_ortho_error_wraps_in_arc(
    invalid_http_timeout_error: InvalidTimeoutError,
) {
    let err =
        Arc::try_unwrap(invalid_http_timeout_error.error).expect("unique ortho_config error Arc");
    let converted: VkError = err.into();
    match converted {
        VkError::Config(stored) => assert_eq!(
            Arc::strong_count(&stored),
            1,
            "owned OrthoError conversion should produce a single-owner Arc"
        ),
        other => panic!("expected VkError::Config, got {other:?}"),
    }
}

#[test]
fn cli_requires_subcommand() {
    assert!(Cli::try_parse_from(["vk"]).is_err());
}

#[test]
fn pr_subcommand_parses() {
    let args = parse_pr_args(&["vk", "pr", "123"]);
    assert_eq!(args.reference.as_deref(), Some("123"));
    assert!(args.files.is_empty());
}

#[test]
fn pr_subcommand_parses_files() {
    let args = parse_pr_args(&["vk", "pr", "123", "src/lib.rs", "README.md"]);
    assert_eq!(args.reference.as_deref(), Some("123"));
    assert_eq!(args.files, ["src/lib.rs", "README.md"]);
}

#[test]
fn pr_subcommand_parses_without_reference() {
    let args = parse_pr_args(&["vk", "pr"]);
    assert!(args.reference.is_none());
    assert!(args.files.is_empty());
}

#[test]
fn pr_subcommand_parses_fragment_only() {
    let args = parse_pr_args(&["vk", "pr", "#discussion_r123"]);
    assert_eq!(args.reference.as_deref(), Some("#discussion_r123"));
}

#[test]
fn pr_subcommand_parses_url() {
    let args = parse_pr_args(&[
        "vk",
        "pr",
        "https://github.com/owner/repo/pull/42#discussion_r99",
    ]);
    assert_eq!(
        args.reference.as_deref(),
        Some("https://github.com/owner/repo/pull/42#discussion_r99")
    );
}

#[test]
fn issue_subcommand_parses() {
    let cli = Cli::try_parse_from(["vk", "issue", "123"]).expect("parse cli");
    match cli.command {
        Commands::Issue(args) => assert_eq!(args.reference.as_deref(), Some("123")),
        _ => panic!("wrong variant"),
    }
}

#[rstest]
#[case(&["vk", "resolve", "83#discussion_r1"], None)]
#[case(&["vk", "resolve", "83#discussion_r1", "-m", "done"], Some("done"))]
fn resolve_subcommand_parses(#[case] argv: &[&str], #[case] expected_message: Option<&str>) {
    let args = parse_resolve_args(argv);
    assert_eq!(args.reference, "83#discussion_r1");
    assert_eq!(args.message.as_deref(), expected_message);
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
        submitted_at: Some(Utc::now()),
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

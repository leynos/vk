//! Tests for the `vk` binary entry point and top-level rendering behaviour.

use super::*;
use crate::printer::{write_comment_body, write_review, write_thread};
use crate::reviews::PullRequestReview;
use crate::test_utils::{remove_var, set_var};
use chrono::Utc;
use ortho_config::OrthoConfig;
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

#[test]
fn cli_loads_repo_from_flag() {
    let cli = Cli::try_parse_from(["vk", "--repo", "foo/bar", "pr", "1"]).expect("parse cli");
    assert_eq!(cli.global.repo.as_deref(), Some("foo/bar"));
}

#[test]
fn cli_loads_github_token_from_flag() {
    let cli = Cli::try_parse_from(["vk", "--github-token", "token", "pr", "1"]).expect("parse cli");
    assert_eq!(cli.global.github_token.as_deref(), Some("token"));
}

fn assert_is_send_sync<T: Send + Sync>() {}

#[test]
fn vk_error_is_send_and_sync() {
    assert_is_send_sync::<VkError>();
}

#[test]
#[serial]
fn vk_error_config_from_arc_preserves_allocation() {
    let old_timeout = environment::var("VK_HTTP_TIMEOUT").ok();
    remove_var("VK_HTTP_TIMEOUT");
    set_var("VK_HTTP_TIMEOUT", "not-a-number");
    let err = GlobalArgs::load_from_iter(std::iter::once(OsString::from("vk")))
        .expect_err("invalid VK_HTTP_TIMEOUT should fail");
    let original = err.clone();
    let converted: VkError = err.into();
    match converted {
        VkError::Config(stored) => assert!(
            Arc::ptr_eq(&stored, &original),
            "conversion from Arc should preserve the original allocation"
        ),
        other => panic!("expected VkError::Config, got {other:?}"),
    }
    match old_timeout {
        Some(v) => set_var("VK_HTTP_TIMEOUT", v),
        None => remove_var("VK_HTTP_TIMEOUT"),
    }
}

#[test]
#[serial]
fn vk_error_config_from_owned_ortho_error_wraps_in_arc() {
    let old_timeout = environment::var("VK_HTTP_TIMEOUT").ok();
    remove_var("VK_HTTP_TIMEOUT");
    set_var("VK_HTTP_TIMEOUT", "not-a-number");
    let err = GlobalArgs::load_from_iter(std::iter::once(OsString::from("vk")))
        .expect_err("invalid VK_HTTP_TIMEOUT should fail");
    let err = Arc::try_unwrap(err).expect("unique ortho_config error Arc");
    let converted: VkError = err.into();
    match converted {
        VkError::Config(stored) => assert_eq!(
            Arc::strong_count(&stored),
            1,
            "owned OrthoError conversion should produce a single-owner Arc"
        ),
        other => panic!("expected VkError::Config, got {other:?}"),
    }
    match old_timeout {
        Some(v) => set_var("VK_HTTP_TIMEOUT", v),
        None => remove_var("VK_HTTP_TIMEOUT"),
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

//! Entry point for the `vk` command line tool.
//!
//! This module defines CLI structure, error types, and the main entry point.
//! Subcommand execution lives in dedicated modules for clarity.

pub mod api;
#[path = "bool_predicates_lib.rs"]
mod bool_predicates;
mod boxed;
mod cli_args;
mod commands;
// configuration helpers have been folded into `ortho_config`
mod auth;
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
pub use review_threads::{
    CommentConnection, FetchOptions, PageInfo, ReviewComment, ReviewThread, User,
    exclude_outdated_threads, fetch_review_threads_with_options, filter_outdated_threads,
    filter_threads_by_files,
};

use crate::cli_args::{GlobalArgs, IssueArgs, PrArgs, ResolveArgs};
use clap::{Parser, Subcommand};
use ortho_config::{OrthoConfig, SubcmdConfigMerge};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::LazyLock;
use thiserror::Error;

pub use auth::resolve_github_token;
use commands::{run_issue, run_pr, run_resolve};

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
    /// Resolve a pull request comment.
    ///
    /// The reference must include a fragment of the form `#discussion_r<ID>`
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

type SharedConfigError = Arc<ortho_config::OrthoError>;

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
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("invalid reference")]
    InvalidRef,
    #[error("GitHub token not set")]
    MissingAuth,
    #[error("pull request number out of range")]
    InvalidNumber,
    #[error("expected URL path segment in {expected:?}, found '{found}'")]
    WrongResourceType {
        expected: &'static [&'static str],
        found: Box<str>,
    },
    #[error("missing comment path at index {index} in thread {thread_id}")]
    EmptyCommentPath { thread_id: Box<str>, index: usize },
    #[error("comment {comment_id} not found")]
    CommentNotFound { comment_id: u64 },
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
    Config(#[from] SharedConfigError),
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

impl From<ortho_config::OrthoError> for VkError {
    fn from(source: ortho_config::OrthoError) -> Self {
        Self::Config(Arc::new(source))
    }
}

pub(crate) static UTF8_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bUTF-?8\b").expect("valid regex"));

#[tokio::main]
async fn main() -> Result<(), VkError> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let Cli {
        command,
        global: global_cli,
    } = Cli::parse();

    let result: Result<(), VkError> = async {
        let mut global = GlobalArgs::load_from_iter(std::env::args_os().take(1))?;
        let cli_token = global_cli.github_token.clone();
        global.merge(global_cli);

        match command {
            Commands::Pr(pr_cli) => {
                let args = pr_cli.load_and_merge()?;
                run_pr(args, &global, cli_token.as_deref()).await
            }
            Commands::Issue(issue_cli) => {
                let args = issue_cli.load_and_merge()?;
                run_issue(args, &global, cli_token.as_deref()).await
            }
            Commands::Resolve(resolve_cli) => {
                let args = resolve_cli.load_and_merge()?;
                run_resolve(args, &global, cli_token.as_deref()).await
            }
        }
    }
    .await;

    if let Err(e) = result {
        eprintln!("Error: {e}");
        let code = match &e {
            VkError::MissingAuth => 2,
            VkError::CommentNotFound { .. } => 3,
            _ => 1,
        };
        std::process::exit(code);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::printer::{write_comment_body, write_review, write_thread};
    use crate::reviews::PullRequestReview;
    use crate::test_utils::{remove_var, set_var};
    use chrono::Utc;
    use serial_test::serial;
    use std::ffi::OsString;
    use std::sync::Arc;
    use termimad::MadSkin;
    use vk::environment;

    #[test]
    fn cli_loads_repo_from_flag() {
        let cli = Cli::try_parse_from(["vk", "--repo", "foo/bar", "pr", "1"]).expect("parse cli");
        assert_eq!(cli.global.repo.as_deref(), Some("foo/bar"));
    }
    #[test]
    fn cli_loads_github_token_from_flag() {
        let cli =
            Cli::try_parse_from(["vk", "--github-token", "token", "pr", "1"]).expect("parse cli");
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
}

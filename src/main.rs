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
mod branch_pr;
mod config_loader;
mod diff;
mod graphql_queries;
mod html;
mod issues;
#[cfg(test)]
mod main_tests;
mod printer;
mod ref_parser;
mod resolve;
mod review_threads;
mod reviews;
mod summary;
#[cfg(test)]
mod test_utils;

mod environment {
    //! Environment helpers for the binary crate.
    pub(crate) use vk::environment::var;
}

pub use crate::api::{GraphQLClient, paginate};
pub use issues::{Issue, fetch_issue};
pub use review_threads::{
    CommentConnection, FetchOptions, PageInfo, ReviewComment, ReviewThread, User,
    exclude_outdated_threads, fetch_review_threads_with_options, filter_outdated_threads,
    filter_threads_by_files,
};

use crate::cli_args::{GlobalArgs, IssueArgs, PrArgs, ResolveArgs};
use clap::{Parser, Subcommand};
use ortho_config::SubcmdConfigMerge;
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
    /// When invoked without arguments, detects the PR associated with the
    /// current Git branch. Passing a `#discussion_r<ID>` fragment shows only
    /// that discussion thread, auto-detecting the PR when no number or URL
    /// is provided. When a fragment is given, both resolved and unresolved
    /// threads are searched. Without a fragment, only unresolved threads are
    /// shown.
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
    #[error("cannot auto-detect PR: repository is in detached HEAD state")]
    DetachedHead,
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
    #[error("no pull request found for branch '{branch}'")]
    NoPrForBranch { branch: Box<str> },
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
        let mut global = config_loader::load_global_args_without_cli_overrides()?;
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

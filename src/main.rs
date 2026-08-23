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

/// Supported top-level command-line subcommands.
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

/// Parsed command-line arguments for the `vk` binary.
#[derive(Debug, Parser)]
#[command(
    name = "vk",
    about = "View Komments - show unresolved PR comments",
    version,
    subcommand_required = true,
    arg_required_else_help = true
)]
struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    command: crate::Commands,
    /// Global options shared by every subcommand.
    #[command(flatten)]
    global: GlobalArgs,
}

/// Shared ownership wrapper for configuration errors.
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
    /// The repository could not be determined from the available references.
    #[error("unable to determine repository")]
    RepoNotFound,
    /// A GitHub request failed before a response could be handled.
    #[error("request failed: {0}")]
    Request(#[from] Box<reqwest::Error>),
    /// A GitHub request failed while running the named operation.
    #[error("request failed when running {context}: {source}")]
    RequestContext {
        /// Operation being performed when the request failed.
        context: Box<str>,
        /// Underlying request error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// A supplied repository or pull-request reference is invalid.
    #[error("invalid reference")]
    InvalidRef,
    /// The repository is in detached HEAD state and has no branch to inspect.
    #[error("cannot auto-detect PR: repository is in detached HEAD state")]
    DetachedHead,
    /// No GitHub token was supplied for an operation that requires one.
    #[error("GitHub token not set")]
    MissingAuth,
    /// A pull-request number could not be parsed or is outside the valid range.
    #[error("pull request number out of range")]
    InvalidNumber,
    /// A URL contained an unexpected resource segment.
    #[error("expected URL path segment in {expected:?}, found '{found}'")]
    WrongResourceType {
        /// Resource segments accepted at this location.
        expected: &'static [&'static str],
        /// Resource segment found in the supplied URL.
        found: Box<str>,
    },
    /// A review thread did not contain the expected comment path.
    #[error("missing comment path at index {index} in thread {thread_id}")]
    EmptyCommentPath {
        /// Review-thread identifier containing the missing path.
        thread_id: Box<str>,
        /// Position at which the path segment was expected.
        index: usize,
    },
    /// A requested review comment was not found.
    #[error("comment {comment_id} not found")]
    CommentNotFound {
        /// Identifier of the missing comment.
        comment_id: u64,
    },
    /// No pull request was found for the supplied branch.
    #[error("no pull request found for branch '{branch}'")]
    NoPrForBranch {
        /// Branch for which no pull request exists.
        branch: Box<str>,
    },
    /// The API returned an unsuccessful response with a textual body.
    #[error("bad response: {0}")]
    BadResponse(Box<str>),
    /// The API returned no data for a successful GraphQL operation.
    #[error("empty GraphQL response (status {status}) for {operation}: {snippet}")]
    EmptyResponse {
        /// HTTP status returned by the API.
        status: u16,
        /// GraphQL operation that produced the response.
        operation: Box<str>,
        /// Bounded response excerpt retained for diagnostics.
        snippet: Box<str>,
    },
    /// The API response could not be deserialized.
    #[error("malformed response (status {status}): {message} | snippet:{snippet}")]
    BadResponseSerde {
        /// HTTP status returned by the API.
        status: u16,
        /// Deserialization error message.
        message: Box<str>,
        /// Bounded response excerpt retained for diagnostics.
        snippet: Box<str>,
    },
    /// The API returned one or more GraphQL errors.
    #[error("API errors: {0}")]
    ApiErrors(Box<str>),
    /// An I/O operation failed.
    #[error("io error: {0}")]
    Io(#[from] Box<std::io::Error>),
    /// Configuration loading or validation failed.
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

/// Pattern that recognizes UTF-8 locale identifiers.
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

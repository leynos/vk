//! Library entry point exposing core functionality.

pub mod api;
pub mod cli_args;
pub mod commands;
pub mod html;
pub mod models;
pub mod printer;
pub mod references;
pub mod reviews;

pub use api::{GraphQLClient, VkError, build_graphql_client, fetch_issue, fetch_review_threads};
pub use references::{RepoInfo, parse_issue_reference, parse_pr_reference};

pub use printer::{
    format_comment_diff, print_end_banner, print_summary, print_thread, summarize_files,
    write_comment, write_comment_body, write_summary, write_thread,
};

pub use commands::{load_with_reference_fallback, locale_is_utf8, run_issue, run_pr};

pub use cli_args::{Cli, Commands, GlobalArgs, IssueArgs, PrArgs};

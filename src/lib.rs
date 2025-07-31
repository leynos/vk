//! Library crate for the `vk` command line tool.

mod api;
pub mod cli_args;
pub mod html;
pub mod printer;
mod ref_utils;
pub mod reviews;

pub use api::{GraphQLClient, build_graphql_client, fetch_issue, fetch_review_threads};
pub use printer::{
    HUNK_RE, format_comment_diff, print_end_banner, print_summary, print_thread, summarize_files,
    write_comment_body, write_summary, write_thread,
};
pub use ref_utils::{
    CommentConnection, Issue, PageInfo, RepoInfo, ReviewComment, ReviewThread, User, VkError,
    load_with_reference_fallback, locale_is_utf8, parse_issue_reference, parse_pr_reference,
};

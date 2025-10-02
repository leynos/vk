//! Library exports for integration tests.
//!
//! Exposes the CLI argument structures so external tests can
//! invoke configuration merging helpers.

pub mod banners;
#[path = "bool_predicates_lib.rs"]
pub mod bool_predicates;
pub mod cli_args;
pub mod environment;
pub mod html;
#[path = "test_utils_env.rs"]
pub mod test_utils;

pub use cli_args::{GlobalArgs, IssueArgs, PrArgs};

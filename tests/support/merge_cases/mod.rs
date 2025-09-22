//! Shared merge precedence fixtures for CLI and subcommand tests.
//!
//! Provides reusable scenario definitions describing how configuration,
//! environment, and CLI inputs interact for each subcommand. Keeping the data
//! here ensures behavioural tests assert the same expectations without
//! duplicating setup logic.

mod data;
mod expectations;

pub use data::{MergeCase, MergeScenario, MergeSubcommand, case};
pub use expectations::MergeExpectation;

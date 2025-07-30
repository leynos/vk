//! Command-line argument structures.
//!
//! Isolates clap derivations so lint expectations remain scoped, keeping
//! `main.rs` focused on runtime logic.
#![expect(non_snake_case, reason = "clap generates non-snake-case modules")]

use clap::Parser;
use clap::Subcommand;
use ortho_config::OrthoConfig;
use serde::{Deserialize, Serialize};

/// Global options that apply to every sub-command (e.g. `--repo`).
#[derive(Parser, Deserialize, Serialize, Default, Debug, OrthoConfig, Clone)]
#[ortho_config(prefix = "VK")]
pub struct GlobalArgs {
    /// Repository used when passing only a pull request number
    #[arg(long)]
    pub repo: Option<String>,
    /// Write HTTP transcript to this file for debugging
    #[arg(long)]
    pub transcript: Option<std::path::PathBuf>,
}

impl GlobalArgs {
    /// Merge another instance into `self`, overwriting only fields that are
    /// currently `None`.
    ///
    /// CLI flags have higher priority than configuration sources.
    pub fn merge(&mut self, other: Self) {
        if let Some(repo) = other.repo {
            self.repo = Some(repo);
        }
        if let Some(transcript) = other.transcript {
            self.transcript = Some(transcript);
        }
    }
}

/// Parameters accepted by the `pr` sub-command.
#[derive(Parser, Deserialize, Serialize, Debug, OrthoConfig, Clone, Default)]
#[ortho_config(prefix = "VK")]
pub struct PrArgs {
    /// Pull request URL or number
    #[arg(required = true)]
    // Clap marks the argument as required so parsing yields `Some(value)`. The
    // `Option` allows `PrArgs::default()` and config merging to leave it unset.
    pub reference: Option<String>,
}

/// Parameters accepted by the `issue` sub-command.
///
/// Stores the URL or number of the issue to inspect.
#[derive(Parser, Deserialize, Serialize, Debug, OrthoConfig, Clone)]
#[ortho_config(prefix = "VK")]
pub struct IssueArgs {
    /// Issue URL or number
    #[arg(required = true)]
    // The argument is required and will parse to `Some`, but `Option` permits
    // defaults or config merging to leave it unset.
    pub reference: Option<String>,
}

#[expect(
    clippy::derivable_impls,
    reason = "manual impl clarifies absent reference state"
)]
impl Default for IssueArgs {
    fn default() -> Self {
        Self { reference: None }
    }
}

#[derive(Subcommand, Deserialize, Serialize, Clone, Debug)]
pub enum Commands {
    /// Show unresolved pull request comments
    Pr(PrArgs),
    /// Read a GitHub issue (todo)
    Issue(IssueArgs),
}

#[derive(Parser)]
#[command(
    name = "vk",
    about = "View Komments - show unresolved PR comments",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
    #[command(flatten)]
    pub global: GlobalArgs,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_requires_subcommand() {
        assert!(Cli::try_parse_from(["vk"]).is_err());
    }

    #[test]
    fn pr_subcommand_parses() {
        let cli = Cli::try_parse_from(["vk", "pr", "123"]).expect("parse cli");
        match cli.command {
            Commands::Pr(args) => assert_eq!(args.reference.as_deref(), Some("123")),
            Commands::Issue(_) => panic!("wrong variant"),
        }
    }

    #[test]
    fn issue_subcommand_parses() {
        let cli = Cli::try_parse_from(["vk", "issue", "123"]).expect("parse cli");
        match cli.command {
            Commands::Issue(args) => assert_eq!(args.reference.as_deref(), Some("123")),
            Commands::Pr(_) => panic!("wrong variant"),
        }
    }

    #[test]
    fn cli_loads_repo_from_flag() {
        let cli = Cli::try_parse_from(["vk", "--repo", "foo/bar", "pr", "1"]).expect("parse cli");
        assert_eq!(cli.global.repo.as_deref(), Some("foo/bar"));
    }
}

//! Command-line argument structures.
//!
//! Isolates clap derivations so lint expectations remain scoped, keeping
//! `main.rs` focused on runtime logic.
#![expect(non_snake_case, reason = "clap generates non-snake-case modules")]
#![expect(unused_imports, reason = "clap derives import the struct internally")]

use clap::Parser;
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
    /// Only show comments for these files
    #[arg(value_name = "FILE", num_args = 0..)]
    #[serde(default)]
    pub files: Vec<String>,
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

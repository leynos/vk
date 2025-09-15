//! Command-line argument structures.
//!
//! Isolates clap derivations so lint expectations remain scoped, keeping
//! `main.rs` focused on runtime logic.
#![allow(non_snake_case, reason = "clap generates non-snake-case modules")]
#![allow(unused_imports, reason = "clap derives import the struct internally")]

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
    /// HTTP request timeout in seconds
    #[arg(long, value_name = "SECS")]
    pub http_timeout: Option<u64>,
    /// HTTP connection timeout in seconds
    #[arg(long, value_name = "SECS")]
    pub connect_timeout: Option<u64>,
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
        if let Some(timeout) = other.http_timeout {
            self.http_timeout = Some(timeout);
        }
        if let Some(timeout) = other.connect_timeout {
            self.connect_timeout = Some(timeout);
        }
    }
}

/// Parameters accepted by the `pr` sub-command.
#[derive(Parser, Deserialize, Serialize, Debug, OrthoConfig, Clone, Default)]
#[command(name = "pr")]
#[ortho_config(prefix = "VK")]
pub struct PrArgs {
    /// Pull request URL or number.
    /// Passing a `#discussion_r<ID>` fragment shows only that discussion thread; file
    /// filters are ignored and unresolved filtering still applies.
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
#[command(name = "issue")]
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

/// Parameters accepted by the `resolve` sub-command.
#[derive(Parser, Deserialize, Serialize, Debug, OrthoConfig, Clone)]
#[command(name = "resolve")]
#[ortho_config(prefix = "VK")]
pub struct ResolveArgs {
    /// Pull request comment URL or number with discussion fragment.
    #[arg(required = true)]
    pub reference: String,
    /// Reply text to post before resolving the comment
    #[arg(
        short = 'm',
        long = "message",
        value_name = "MESSAGE",
        help = "Reply text to post before resolving the comment"
    )]
    pub message: Option<String>,
}

#[expect(
    clippy::derivable_impls,
    reason = "manual impl clarifies default empty reference"
)]
impl Default for ResolveArgs {
    fn default() -> Self {
        Self {
            reference: String::new(),
            message: None,
        }
    }
}

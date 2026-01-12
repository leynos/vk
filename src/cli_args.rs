//! Command-line argument structures.
//!
//! Isolates clap derivations so lint expectations remain scoped, keeping
//! `main.rs` focused on runtime logic.
// Imports are referenced by derives; no suppression required.

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
    /// GitHub token for authenticated API requests
    #[arg(long, value_name = "TOKEN")]
    pub github_token: Option<String>,
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
        self.repo = other.repo.or_else(|| self.repo.take());
        self.github_token = other.github_token.or_else(|| self.github_token.take());
        self.transcript = other.transcript.or_else(|| self.transcript.take());
        self.http_timeout = other.http_timeout.or_else(|| self.http_timeout.take());
        self.connect_timeout = other
            .connect_timeout
            .or_else(|| self.connect_timeout.take());
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// Include outdated review threads
    #[arg(short = 'o', long = "show-outdated")]
    // `crate::bool_predicates::not` ensures false CLI defaults cannot override env or config precedence.
    #[serde(
        default,
        alias = "include_outdated",
        skip_serializing_if = "crate::bool_predicates::not"
    )]
    pub show_outdated: bool,
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

#[cfg(test)]
mod tests {
    use super::GlobalArgs;

    #[test]
    fn merge_prefers_cli_github_token() {
        let mut config = GlobalArgs {
            github_token: Some("config-token".to_string()),
            ..GlobalArgs::default()
        };
        let cli = GlobalArgs {
            github_token: Some("cli-token".to_string()),
            ..GlobalArgs::default()
        };

        config.merge(cli);

        assert_eq!(config.github_token.as_deref(), Some("cli-token"));
    }

    #[test]
    fn merge_keeps_config_github_token_when_cli_missing() {
        let mut config = GlobalArgs {
            github_token: Some("config-token".to_string()),
            ..GlobalArgs::default()
        };
        let cli = GlobalArgs::default();

        config.merge(cli);

        assert_eq!(config.github_token.as_deref(), Some("config-token"));
    }
}

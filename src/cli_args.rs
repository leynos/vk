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
    /// Include outdated review threads
    #[arg(short = 'o', long = "show-outdated")]
    #[serde(default, alias = "include_outdated")]
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
    use super::*;
    use crate::test_utils::{remove_var, set_var};
    use ortho_config::SubcmdConfigMerge;
    use serial_test::serial;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    struct EnvGuard {
        keys: Vec<&'static str>,
    }

    impl EnvGuard {
        fn new(keys: &[&'static str]) -> Self {
            for key in keys {
                remove_var(key);
            }
            Self {
                keys: keys.to_vec(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for key in &self.keys {
                remove_var(key);
            }
        }
    }

    fn write_config(content: &str) -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("create config dir");
        let path = dir.path().join(".vk.toml");
        fs::write(&path, content).expect("write config");
        (dir, path)
    }

    #[test]
    #[serial]
    fn load_and_merge_prefers_cli_over_other_sources() {
        let _guard = EnvGuard::new(&[
            "VK_CONFIG_PATH",
            "VKCMDS_PR_REFERENCE",
            "VKCMDS_PR_FILES",
            "VKCMDS_PR_SHOW_OUTDATED",
        ]);

        let cfg = r#"[cmds.pr]
reference = "file_ref"
files = ["config.txt"]
show_outdated = false
"#;
        let (_config_dir, config_path) = write_config(cfg);
        set_var("VK_CONFIG_PATH", config_path.as_os_str());
        set_var("VKCMDS_PR_REFERENCE", "env_ref");
        set_var("VKCMDS_PR_FILES", "env.txt");
        set_var("VKCMDS_PR_SHOW_OUTDATED", "false");

        let cli = PrArgs {
            reference: Some(String::from("cli_ref")),
            files: vec![String::from("cli.txt")],
            show_outdated: true,
        };

        let merged = cli.load_and_merge().expect("merge pr args");

        assert_eq!(merged.reference.as_deref(), Some("cli_ref"));
        assert_eq!(merged.files, vec![String::from("cli.txt")]);
        assert!(merged.show_outdated);
    }

    #[test]
    #[serial]
    fn load_and_merge_uses_environment_reference() {
        let _guard = EnvGuard::new(&[
            "VK_CONFIG_PATH",
            "VKCMDS_PR_REFERENCE",
            "VKCMDS_PR_FILES",
            "VKCMDS_PR_SHOW_OUTDATED",
        ]);

        let cfg = r#"[cmds.pr]
reference = "file_ref"
files = ["config.txt"]
show_outdated = false
"#;
        let (_config_dir, config_path) = write_config(cfg);
        set_var("VK_CONFIG_PATH", config_path.as_os_str());
        set_var("VKCMDS_PR_REFERENCE", "env_ref");
        set_var("VKCMDS_PR_FILES", "env_one.rs,env_two.rs");
        set_var("VKCMDS_PR_SHOW_OUTDATED", "1");

        let cli = PrArgs::default();
        let merged = cli.load_and_merge().expect("merge pr args");

        // Only the optional reference can be filled from the environment. Clap
        // initialises vectors and booleans eagerly, so their defaults read as
        // explicit CLI choices and we leave them untouched by config or
        // environment overrides.

        assert_eq!(merged.reference.as_deref(), Some("env_ref"));
        assert!(merged.files.is_empty());
        assert!(!merged.show_outdated);
    }

    #[test]
    #[serial]
    fn load_and_merge_retains_none_without_sources() {
        let _guard = EnvGuard::new(&[
            "VK_CONFIG_PATH",
            "VKCMDS_PR_REFERENCE",
            "VKCMDS_PR_FILES",
            "VKCMDS_PR_SHOW_OUTDATED",
        ]);

        let cfg = "";
        let (_config_dir, config_path) = write_config(cfg);
        set_var("VK_CONFIG_PATH", config_path.as_os_str());

        let cli = PrArgs::default();
        let merged = cli.load_and_merge().expect("merge pr args");

        assert!(merged.reference.is_none());
        assert!(!merged.show_outdated);
    }

    #[test]
    #[serial]
    fn load_and_merge_preserves_cli_instance() {
        let _guard = EnvGuard::new(&[
            "VK_CONFIG_PATH",
            "VKCMDS_PR_REFERENCE",
            "VKCMDS_PR_FILES",
            "VKCMDS_PR_SHOW_OUTDATED",
        ]);

        let cli = PrArgs {
            reference: Some(String::from("cli_ref")),
            files: vec![String::from("cli.txt")],
            show_outdated: true,
        };
        let snapshot = cli.clone();

        let merged = cli.load_and_merge().expect("merge pr args");

        assert_eq!(cli.reference, snapshot.reference);
        assert_eq!(cli.files, snapshot.files);
        assert_eq!(cli.show_outdated, snapshot.show_outdated);
        assert_eq!(merged.reference.as_deref(), Some("cli_ref"));
        assert_eq!(merged.files, vec![String::from("cli.txt")]);
        assert!(merged.show_outdated);
    }

    #[test]
    #[serial]
    fn load_and_merge_prefers_cli_issue_reference() {
        let _guard = EnvGuard::new(&["VK_CONFIG_PATH", "VKCMDS_ISSUE_REFERENCE"]);

        let cfg = r#"[cmds.issue]
reference = "file_ref"
"#;
        let (_config_dir, config_path) = write_config(cfg);
        set_var("VK_CONFIG_PATH", config_path.as_os_str());
        set_var("VKCMDS_ISSUE_REFERENCE", "env_ref");

        let cli = IssueArgs {
            reference: Some(String::from("cli_ref")),
        };

        let merged = cli.load_and_merge().expect("merge issue args");

        assert_eq!(merged.reference.as_deref(), Some("cli_ref"));
    }

    #[test]
    #[serial]
    fn load_and_merge_uses_issue_environment_reference() {
        let _guard = EnvGuard::new(&["VK_CONFIG_PATH", "VKCMDS_ISSUE_REFERENCE"]);

        let cfg = r#"[cmds.issue]
reference = "file_ref"
"#;
        let (_config_dir, config_path) = write_config(cfg);
        set_var("VK_CONFIG_PATH", config_path.as_os_str());
        set_var("VKCMDS_ISSUE_REFERENCE", "env_ref");

        let cli = IssueArgs::default();
        let merged = cli.load_and_merge().expect("merge issue args");

        assert_eq!(merged.reference.as_deref(), Some("env_ref"));
    }

    #[test]
    #[serial]
    fn load_and_merge_uses_issue_config_reference() {
        let _guard = EnvGuard::new(&["VK_CONFIG_PATH", "VKCMDS_ISSUE_REFERENCE"]);

        let cfg = r#"[cmds.issue]
reference = "file_ref"
"#;
        let (_config_dir, config_path) = write_config(cfg);
        set_var("VK_CONFIG_PATH", config_path.as_os_str());

        let cli = IssueArgs::default();
        // Change into the config directory so the default `.vk.toml` is discovered.
        let prev_dir = std::env::current_dir().expect("current dir");
        let config_dir = config_path.parent().expect("config dir");
        std::env::set_current_dir(config_dir).expect("set dir");
        let merged = cli.load_and_merge().expect("merge issue args");
        std::env::set_current_dir(prev_dir).expect("restore dir");

        assert_eq!(merged.reference.as_deref(), Some("file_ref"));
    }

    #[test]
    #[serial]
    fn load_and_merge_preserves_issue_cli_instance() {
        let _guard = EnvGuard::new(&["VK_CONFIG_PATH", "VKCMDS_ISSUE_REFERENCE"]);

        let cli = IssueArgs {
            reference: Some(String::from("cli_ref")),
        };
        let snapshot = cli.clone();

        let merged = cli.load_and_merge().expect("merge issue args");

        assert_eq!(cli.reference, snapshot.reference);
        assert_eq!(merged.reference.as_deref(), Some("cli_ref"));
    }

    #[test]
    #[serial]
    fn load_and_merge_prefers_cli_resolve_values() {
        let _guard = EnvGuard::new(&[
            "VK_CONFIG_PATH",
            "VKCMDS_RESOLVE_REFERENCE",
            "VKCMDS_RESOLVE_MESSAGE",
        ]);

        let cfg = r#"[cmds.resolve]
reference = "file_ref"
message = "file message"
"#;
        let (_config_dir, config_path) = write_config(cfg);
        set_var("VK_CONFIG_PATH", config_path.as_os_str());
        set_var("VKCMDS_RESOLVE_REFERENCE", "env_ref");
        set_var("VKCMDS_RESOLVE_MESSAGE", "env message");

        let cli = ResolveArgs {
            reference: String::from("cli_ref"),
            message: Some(String::from("cli message")),
        };

        let merged = cli.load_and_merge().expect("merge resolve args");

        assert_eq!(merged.reference, "cli_ref");
        assert_eq!(merged.message.as_deref(), Some("cli message"));
    }

    #[test]
    #[serial]
    fn load_and_merge_uses_resolve_environment_message() {
        let _guard = EnvGuard::new(&[
            "VK_CONFIG_PATH",
            "VKCMDS_RESOLVE_REFERENCE",
            "VKCMDS_RESOLVE_MESSAGE",
        ]);

        let cfg = r#"[cmds.resolve]
message = "file message"
"#;
        let (_config_dir, config_path) = write_config(cfg);
        set_var("VK_CONFIG_PATH", config_path.as_os_str());
        set_var("VKCMDS_RESOLVE_MESSAGE", "env message");
        set_var("VKCMDS_RESOLVE_REFERENCE", "env_ref");

        let cli = ResolveArgs {
            reference: String::from("cli_ref"),
            message: None,
        };

        let merged = cli.load_and_merge().expect("merge resolve args");

        assert_eq!(merged.reference, "cli_ref");
        assert_eq!(merged.message.as_deref(), Some("env message"));
    }

    #[test]
    #[serial]
    fn load_and_merge_uses_resolve_config_message() {
        let _guard = EnvGuard::new(&[
            "VK_CONFIG_PATH",
            "VKCMDS_RESOLVE_REFERENCE",
            "VKCMDS_RESOLVE_MESSAGE",
        ]);

        let cfg = r#"[cmds.resolve]
message = "file message"
"#;
        let (_config_dir, config_path) = write_config(cfg);
        set_var("VK_CONFIG_PATH", config_path.as_os_str());

        let cli = ResolveArgs {
            reference: String::from("cli_ref"),
            message: None,
        };

        // Change into the config directory so the default `.vk.toml` is discovered.
        let prev_dir = std::env::current_dir().expect("current dir");
        let config_dir = config_path.parent().expect("config dir");
        std::env::set_current_dir(config_dir).expect("set dir");
        let merged = cli.load_and_merge().expect("merge resolve args");
        std::env::set_current_dir(prev_dir).expect("restore dir");

        assert_eq!(merged.reference, "cli_ref");
        assert_eq!(merged.message.as_deref(), Some("file message"));
    }

    #[test]
    #[serial]
    fn load_and_merge_preserves_resolve_cli_instance() {
        let _guard = EnvGuard::new(&[
            "VK_CONFIG_PATH",
            "VKCMDS_RESOLVE_REFERENCE",
            "VKCMDS_RESOLVE_MESSAGE",
        ]);

        let cli = ResolveArgs {
            reference: String::from("cli_ref"),
            message: Some(String::from("cli message")),
        };
        let snapshot = cli.clone();

        let merged = cli.load_and_merge().expect("merge resolve args");

        assert_eq!(cli.reference, snapshot.reference);
        assert_eq!(cli.message, snapshot.message);
        assert_eq!(merged.reference, "cli_ref");
        assert_eq!(merged.message.as_deref(), Some("cli message"));
    }
}

//! Tests verifying configuration precedence for subcommands.
//!
//! These tests ensure that defaults from configuration files can be overridden
//! by environment variables and, finally, by command-line arguments.

use std::{env, fs, path::PathBuf};

use ortho_config::subcommand::load_and_merge_subcommand_for;
use rstest::rstest;
use serial_test::serial;
use tempfile::TempDir;
use vk::{IssueArgs, PrArgs};

/// Write `content` to a temporary `.vk.toml` file and return its directory.
fn write_config(content: &str) -> TempDir {
    let dir = tempfile::tempdir().expect("create config dir");
    fs::write(dir.path().join(".vk.toml"), content).expect("write config");
    dir
}

/// Set or clear an environment variable.
fn set_env(key: &str, value: Option<&str>) {
    if let Some(v) = value {
        // Safety: tests run serially so environment writes are isolated.
        unsafe {
            env::set_var(key, v);
        }
    } else {
        // Safety: tests run serially so environment writes are isolated.
        unsafe {
            env::remove_var(key);
        }
    }
}

/// RAII guard restoring the working directory on drop.
struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        env::set_current_dir(&self.0).expect("restore dir");
    }
}

/// Change into `dir`, returning a guard that restores the previous directory.
fn set_dir(dir: &TempDir) -> DirGuard {
    let prev = env::current_dir().expect("current dir");
    env::set_current_dir(dir.path()).expect("change dir");
    DirGuard(prev)
}

#[rstest]
#[serial]
fn pr_configuration_precedence() {
    let cfg = "[cmds.pr]\nreference = \"file_ref\"\nfiles = [\"file.txt\"]\n";
    let dir = write_config(cfg);
    let _guard = set_dir(&dir);
    set_env("VK_CMDS_PR_REFERENCE", Some("env_ref"));
    set_env("VK_CMDS_PR_FILES", Some("env.txt"));
    let cli = PrArgs {
        reference: Some("cli_ref".into()),
        files: vec!["cli.txt".into()],
    };
    let merged = load_and_merge_subcommand_for(&cli).expect("merge pr args");
    assert_eq!(merged.reference.as_deref(), Some("cli_ref"));
    assert_eq!(merged.files, ["cli.txt"]);
    set_env("VK_CMDS_PR_REFERENCE", None);
    set_env("VK_CMDS_PR_FILES", None);
}

#[rstest]
#[serial]
fn issue_configuration_precedence() {
    let cfg = "[cmds.issue]\nreference = \"file_ref\"\n";
    let dir = write_config(cfg);
    let _guard = set_dir(&dir);
    set_env("VK_CMDS_ISSUE_REFERENCE", Some("env_ref"));
    let cli = IssueArgs {
        reference: Some("cli_ref".into()),
    };
    let merged = load_and_merge_subcommand_for(&cli).expect("merge issue args");
    assert_eq!(merged.reference.as_deref(), Some("cli_ref"));
    set_env("VK_CMDS_ISSUE_REFERENCE", None);
}

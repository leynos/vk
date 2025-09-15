//! Tests verifying configuration precedence for subcommands.
//!
//! These tests ensure that defaults from configuration files can be overridden
//! by environment variables and, finally, by command-line arguments.

use std::{env, fs, path::PathBuf};

use ortho_config::subcommand::load_and_merge_subcommand_for;
use rstest::rstest;
use serial_test::serial;
use tempfile::TempDir;
use vk::test_utils::{remove_var, set_var};
use vk::{IssueArgs, PrArgs};

/// Write `content` to a temporary `vk.toml` file and return its directory.
fn write_config(content: &str) -> TempDir {
    let dir = tempfile::tempdir().expect("create config dir");
    fs::write(dir.path().join("vk.toml"), content).expect("write config");
    dir
}

/// RAII guard restoring the working directory on drop.
struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.0); // best-effort restore; Drop must not panic
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
    set_var("VK_CMDS_PR_REFERENCE", "env_ref");
    set_var("VK_CMDS_PR_FILES", "env.txt");
    let cli = PrArgs {
        reference: Some("cli_ref".into()),
        files: vec!["cli.txt".into()],
    };
    let merged = load_and_merge_subcommand_for(&cli).expect("merge pr args");
    assert_eq!(merged.reference.as_deref(), Some("cli_ref"));
    assert_eq!(merged.files, ["cli.txt"]);
    remove_var("VK_CMDS_PR_REFERENCE");
    remove_var("VK_CMDS_PR_FILES");
}

#[rstest]
#[serial]
fn issue_configuration_precedence() {
    let cfg = "[cmds.issue]\nreference = \"file_ref\"\n";
    let dir = write_config(cfg);
    let _guard = set_dir(&dir);
    set_var("VK_CMDS_ISSUE_REFERENCE", "env_ref");
    let cli = IssueArgs {
        reference: Some("cli_ref".into()),
    };
    let merged = load_and_merge_subcommand_for(&cli).expect("merge issue args");
    assert_eq!(merged.reference.as_deref(), Some("cli_ref"));
    remove_var("VK_CMDS_ISSUE_REFERENCE");
}

#[rstest]
#[serial]
#[ignore = "requires config path setup"]
fn issue_configuration_fallback_to_config_only() {
    let cfg = r#"[cmds.issue]
reference = "file_ref"
"#;
    let dir = write_config(cfg);
    let cfg_path = dir.path().join("vk.toml");
    set_var(
        "VK_CMDS_ISSUE_CONFIG_PATH",
        cfg_path.to_str().expect("cfg path"),
    );
    remove_var("VK_CMDS_ISSUE_REFERENCE");
    let cli = IssueArgs { reference: None };
    let merged = load_and_merge_subcommand_for(&cli).expect("merge issue args");
    assert_eq!(merged.reference.as_deref(), Some("file_ref"));
    remove_var("VK_CMDS_ISSUE_CONFIG_PATH");
}

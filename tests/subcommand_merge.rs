//! Tests verifying configuration precedence for subcommands.
//!
//! These tests ensure configuration file values override defaults, are overridden
//! by environment variables, and finally by command-line arguments.

use std::{env, fs, path::PathBuf};

use clap::CommandFactory;
use ortho_config::subcommand::load_and_merge_subcommand_for;
use rstest::rstest;
use serial_test::serial;
use tempfile::TempDir;
use vk::test_utils::{remove_var, set_var};
use vk::{IssueArgs, PrArgs};

/// Write `content` to a temporary `.vk.toml` file and return its directory.
fn write_config(content: &str) -> TempDir {
    let dir = tempfile::tempdir().expect("create config dir");
    fs::write(dir.path().join(".vk.toml"), content).expect("write config");
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

fn merge_with_sources<T>(config: &str, env: &[(&str, Option<&str>)], cli: &T) -> T
where
    T: ortho_config::OrthoConfig + serde::Serialize + Default + CommandFactory,
{
    let dir = write_config(config);
    let _guard = set_dir(&dir);
    let config_path = dir.path().join(".vk.toml");
    set_var("VK_CONFIG_PATH", config_path.as_os_str());

    for (key, value) in env {
        match value {
            Some(v) => set_var(key, v),
            None => remove_var(key),
        }
    }

    let merged = load_and_merge_subcommand_for(cli)
        .unwrap_or_else(|err| panic!("merge {} args: {err}", std::any::type_name::<T>()));

    for (key, _) in env {
        remove_var(key);
    }

    remove_var("VK_CONFIG_PATH");

    merged
}

#[rstest]
#[serial]
fn pr_configuration_precedence() {
    let cfg = r#"[cmds.pr]
reference = "file_ref"
files = ["file.txt"]
"#;
    let merged = merge_with_sources(
        cfg,
        &[
            ("VKCMDS_PR_REFERENCE", Some("env_ref")),
            ("VKCMDS_PR_FILES", Some("env.txt")),
        ],
        &PrArgs {
            reference: Some("cli_ref".into()),
            files: vec!["cli.txt".into()],
        },
    );
    assert_eq!(merged.reference.as_deref(), Some("cli_ref"));
}

#[rstest]
#[serial]
fn issue_configuration_precedence() {
    let cfg = r#"[cmds.issue]
reference = "file_ref"
"#;
    let merged = merge_with_sources(
        cfg,
        &[("VKCMDS_ISSUE_REFERENCE", Some("env_ref"))],
        &IssueArgs {
            reference: Some("cli_ref".into()),
        },
    );
    assert_eq!(merged.reference.as_deref(), Some("cli_ref"));
}

#[rstest]
#[serial]
fn pr_env_over_file_when_cli_absent() {
    let cfg = r#"[cmds.pr]
reference = "file_ref"
files = ["file.txt"]
"#;
    let merged = merge_with_sources(
        cfg,
        &[
            ("VKCMDS_PR_REFERENCE", Some("env_ref")),
            ("VKCMDS_PR_FILES", None),
        ],
        &PrArgs {
            reference: None,
            files: vec![],
        },
    );
    assert_eq!(merged.reference.as_deref(), Some("env_ref"));
}

#[rstest]
#[serial]
fn pr_file_over_defaults_when_env_and_cli_absent() {
    let cfg = r#"[cmds.pr]
reference = "file_ref"
files = ["file.txt"]
"#;
    let merged = merge_with_sources(
        cfg,
        &[("VKCMDS_PR_REFERENCE", None), ("VKCMDS_PR_FILES", None)],
        &PrArgs {
            reference: None,
            files: vec![],
        },
    );
    assert_eq!(merged.reference.as_deref(), Some("file_ref"));
}

#[rstest]
#[serial]
fn issue_env_over_file_when_cli_absent() {
    let cfg = r#"[cmds.issue]
reference = "file_ref"
"#;
    let merged = merge_with_sources(
        cfg,
        &[("VKCMDS_ISSUE_REFERENCE", Some("env_ref"))],
        &IssueArgs { reference: None },
    );
    assert_eq!(merged.reference.as_deref(), Some("env_ref"));
}

#[rstest]
#[serial]
fn issue_file_over_defaults_when_env_and_cli_absent() {
    let cfg = r#"[cmds.issue]
reference = "file_ref"
"#;
    let merged = merge_with_sources(
        cfg,
        &[("VKCMDS_ISSUE_REFERENCE", None)],
        &IssueArgs { reference: None },
    );
    assert_eq!(merged.reference.as_deref(), Some("file_ref"));
}

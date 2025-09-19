//! Tests verifying configuration precedence for subcommands.
//!
//! These tests ensure configuration file values override defaults, are overridden
//! by environment variables, and finally by command-line arguments.

use std::{env, fs, path::PathBuf};

use clap::CommandFactory;
use ortho_config::SubcmdConfigMerge;
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

    let merged = cli
        .load_and_merge()
        .unwrap_or_else(|err| panic!("merge {} args: {err}", std::any::type_name::<T>()));

    for (key, _) in env {
        remove_var(key);
    }

    remove_var("VK_CONFIG_PATH");

    merged
}

fn pr_cli(reference: Option<&str>, files: &[&str]) -> PrArgs {
    let mut args = PrArgs::default();
    if let Some(reference) = reference {
        args.reference = Some(reference.to_owned());
    }
    if !files.is_empty() {
        args.files = files.iter().map(|value| (*value).to_owned()).collect();
    }
    args
}

fn issue_cli(reference: Option<&str>) -> IssueArgs {
    let mut args = IssueArgs::default();
    if let Some(reference) = reference {
        args.reference = Some(reference.to_owned());
    }
    args
}

#[derive(Copy, Clone, Debug)]
enum SubcommandType {
    Pr,
    Issue,
}

#[derive(Copy, Clone, Debug)]
enum PrecedenceScenario {
    CliOverEnv,
    EnvOverFile,
    FileOverDefaults,
}

#[rstest]
#[case(SubcommandType::Pr, PrecedenceScenario::CliOverEnv)]
#[case(SubcommandType::Pr, PrecedenceScenario::EnvOverFile)]
#[case(SubcommandType::Pr, PrecedenceScenario::FileOverDefaults)]
#[case(SubcommandType::Issue, PrecedenceScenario::CliOverEnv)]
#[case(SubcommandType::Issue, PrecedenceScenario::EnvOverFile)]
#[case(SubcommandType::Issue, PrecedenceScenario::FileOverDefaults)]
#[serial]
fn test_configuration_precedence(
    #[case] subcommand: SubcommandType,
    #[case] scenario: PrecedenceScenario,
) {
    match (subcommand, scenario) {
        (SubcommandType::Pr, PrecedenceScenario::CliOverEnv) => {
            let cfg = r#"[cmds.pr]
reference = "file_ref"
files = ["file.txt"]
"#;
            let cli = pr_cli(Some("cli_ref"), &["cli.txt"]);
            let merged = merge_with_sources(cfg, &[("VKCMDS_PR_REFERENCE", Some("env_ref"))], &cli);
            assert_eq!(merged.reference.as_deref(), Some("cli_ref"));
        }
        (SubcommandType::Pr, PrecedenceScenario::EnvOverFile) => {
            let cfg = r#"[cmds.pr]
reference = "file_ref"
files = ["file.txt"]
"#;
            let cli = pr_cli(None, &[]);
            let merged = merge_with_sources(cfg, &[("VKCMDS_PR_REFERENCE", Some("env_ref"))], &cli);
            assert_eq!(merged.reference.as_deref(), Some("env_ref"));
        }
        (SubcommandType::Pr, PrecedenceScenario::FileOverDefaults) => {
            let cfg = r#"[cmds.pr]
reference = "file_ref"
files = ["file.txt"]
"#;
            let cli = pr_cli(None, &[]);
            let merged = merge_with_sources(
                cfg,
                &[("VKCMDS_PR_REFERENCE", None), ("VKCMDS_PR_FILES", None)],
                &cli,
            );
            assert_eq!(merged.reference.as_deref(), Some("file_ref"));
        }
        (SubcommandType::Issue, PrecedenceScenario::CliOverEnv) => {
            let cfg = r#"[cmds.issue]
reference = "file_ref"
"#;
            let cli = issue_cli(Some("cli_ref"));
            let merged =
                merge_with_sources(cfg, &[("VKCMDS_ISSUE_REFERENCE", Some("env_ref"))], &cli);
            assert_eq!(merged.reference.as_deref(), Some("cli_ref"));
        }
        (SubcommandType::Issue, PrecedenceScenario::EnvOverFile) => {
            let cfg = r#"[cmds.issue]
reference = "file_ref"
"#;
            let cli = issue_cli(None);
            let merged =
                merge_with_sources(cfg, &[("VKCMDS_ISSUE_REFERENCE", Some("env_ref"))], &cli);
            assert_eq!(merged.reference.as_deref(), Some("env_ref"));
        }
        (SubcommandType::Issue, PrecedenceScenario::FileOverDefaults) => {
            let cfg = r#"[cmds.issue]
reference = "file_ref"
"#;
            let cli = issue_cli(None);
            let merged = merge_with_sources(cfg, &[("VKCMDS_ISSUE_REFERENCE", None)], &cli);
            assert_eq!(merged.reference.as_deref(), Some("file_ref"));
        }
    }
}

#[test]
#[serial]
fn pr_load_and_merge_uses_environment_reference() {
    let cfg = r#"[cmds.pr]
reference = "file_ref"
files = ["config.txt"]
show_outdated = false
"#;
    let cli = pr_cli(None, &[]);
    let merged = merge_with_sources(
        cfg,
        &[
            ("VKCMDS_PR_REFERENCE", Some("env_ref")),
            ("VKCMDS_PR_FILES", Some("env.rs,extra.rs")),
            ("VKCMDS_PR_SHOW_OUTDATED", Some("1")),
        ],
        &cli,
    );
    assert_eq!(merged.reference.as_deref(), Some("env_ref"));
    assert!(merged.files.is_empty());
    assert!(!merged.show_outdated);
}

#[test]
#[serial]
fn pr_load_and_merge_cli_values_override_env() {
    let cfg = r#"[cmds.pr]
reference = "file_ref"
files = ["config.txt"]
show_outdated = false
"#;
    let mut cli = pr_cli(Some("cli_ref"), &["cli.txt"]);
    cli.show_outdated = true;
    let merged = merge_with_sources(
        cfg,
        &[
            ("VKCMDS_PR_REFERENCE", Some("env_ref")),
            ("VKCMDS_PR_FILES", Some("env.txt")),
            ("VKCMDS_PR_SHOW_OUTDATED", Some("false")),
        ],
        &cli,
    );
    assert_eq!(merged.reference.as_deref(), Some("cli_ref"));
    assert_eq!(merged.files, vec![String::from("cli.txt")]);
    assert!(merged.show_outdated);
}

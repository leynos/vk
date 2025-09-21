//! Tests verifying configuration precedence for subcommands.
//!
//! These tests ensure configuration file values override defaults, are overridden
//! by environment variables, and finally by command-line arguments.

#[path = "support/env.rs"]
mod env_support;
#[path = "support/subcommand.rs"]
mod sub_support;

use rstest::rstest;
use serial_test::serial;
use sub_support::{issue_cli, merge_with_sources, pr_cli, resolve_cli};
use vk::PrArgs;

#[derive(Copy, Clone, Debug)]
enum SubcommandType {
    Pr,
    Issue,
    Resolve,
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
#[case(SubcommandType::Resolve, PrecedenceScenario::CliOverEnv)]
#[case(SubcommandType::Resolve, PrecedenceScenario::EnvOverFile)]
#[case(SubcommandType::Resolve, PrecedenceScenario::FileOverDefaults)]
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
                &[
                    ("VKCMDS_PR_REFERENCE", None),
                    ("VKCMDS_PR_FILES", None),
                    ("VKCMDS_PR_SHOW_OUTDATED", None),
                ],
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
        (SubcommandType::Resolve, PrecedenceScenario::CliOverEnv) => {
            let cfg = r#"[cmds.resolve]
reference = "file_ref"
message = "file message"
"#;
            let cli = resolve_cli("cli_ref", Some("cli message"));
            let merged = merge_with_sources(
                cfg,
                &[
                    ("VKCMDS_RESOLVE_REFERENCE", Some("env_ref")),
                    ("VKCMDS_RESOLVE_MESSAGE", Some("env message")),
                ],
                &cli,
            );
            assert_eq!(merged.reference, "cli_ref");
            assert_eq!(merged.message.as_deref(), Some("cli message"));
        }
        (SubcommandType::Resolve, PrecedenceScenario::EnvOverFile) => {
            let cfg = r#"[cmds.resolve]
message = "file message"
"#;
            let cli = resolve_cli("cli_ref", None);
            let merged = merge_with_sources(
                cfg,
                &[
                    ("VKCMDS_RESOLVE_REFERENCE", None),
                    ("VKCMDS_RESOLVE_MESSAGE", Some("env message")),
                ],
                &cli,
            );
            assert_eq!(merged.reference, "cli_ref");
            assert_eq!(merged.message.as_deref(), Some("env message"));
        }
        (SubcommandType::Resolve, PrecedenceScenario::FileOverDefaults) => {
            let cfg = r#"[cmds.resolve]
message = "file message"
"#;
            let cli = resolve_cli("cli_ref", None);
            let merged = merge_with_sources(
                cfg,
                &[
                    ("VKCMDS_RESOLVE_REFERENCE", None),
                    ("VKCMDS_RESOLVE_MESSAGE", None),
                ],
                &cli,
            );
            assert_eq!(merged.reference, "cli_ref");
            assert_eq!(merged.message.as_deref(), Some("file message"));
        }
    }
}

#[derive(Debug)]
struct PrMergeTestCase {
    cli: PrArgs,
    env: &'static [(&'static str, Option<&'static str>)],
    expected_reference: Option<&'static str>,
    expected_files: &'static [&'static str],
    expected_show_outdated: bool,
}

#[rstest]
#[case::uses_environment_reference(PrMergeTestCase {
    cli: pr_cli(None, &[]),
    env: &[
        ("VKCMDS_PR_REFERENCE", Some("env_ref")),
        ("VKCMDS_PR_FILES", Some("env.rs,extra.rs")),
        ("VKCMDS_PR_SHOW_OUTDATED", Some("1")),
    ],
    expected_reference: Some("env_ref"),
    expected_files: &[],
    expected_show_outdated: false,
})]
#[case::cli_values_override_environment({
    let mut cli = pr_cli(Some("cli_ref"), &["cli.txt"]);
    cli.show_outdated = true;
    PrMergeTestCase {
        cli,
        env: &[
            ("VKCMDS_PR_REFERENCE", Some("env_ref")),
            ("VKCMDS_PR_FILES", Some("env.txt")),
            ("VKCMDS_PR_SHOW_OUTDATED", Some("false")),
        ],
        expected_reference: Some("cli_ref"),
        expected_files: &["cli.txt"],
        expected_show_outdated: true,
    }
})]
#[serial]
fn test_pr_load_and_merge_precedence(#[case] case: PrMergeTestCase) {
    let config = r#"[cmds.pr]
reference = "file_ref"
files = ["config.txt"]
show_outdated = false
"#;

    let PrMergeTestCase {
        cli,
        env,
        expected_reference,
        expected_files,
        expected_show_outdated,
    } = case;

    let merged = merge_with_sources(config, env, &cli);

    assert_eq!(merged.reference.as_deref(), expected_reference);
    assert_eq!(
        merged.files,
        expected_files
            .iter()
            .map(|value| String::from(*value))
            .collect::<Vec<_>>()
    );
    assert_eq!(merged.show_outdated, expected_show_outdated);
}

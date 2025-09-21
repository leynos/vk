//! Behavioural coverage for CLI argument merging helpers.

#[path = "support/env.rs"]
mod support;

use ortho_config::SubcmdConfigMerge;
use rstest::{fixture, rstest};
use serial_test::serial;
use support::{DirGuard, EnvGuard, setup_env_and_config};
use vk::cli_args::{IssueArgs, PrArgs, ResolveArgs};
use vk::test_utils::set_var;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubcommandType {
    Pr,
    Issue,
    Resolve,
}

#[derive(Debug)]
struct TestScenario {
    subcommand: SubcommandType,
    env_vars: &'static [&'static str],
    config_section: &'static str,
}

#[derive(Debug, Clone, Copy)]
enum ConfigVariant {
    CliDominant,
    EnvFallback,
    ConfigFallback,
}

trait RefStr {
    fn reference_str(&self) -> Option<&str>;
}

impl RefStr for PrArgs {
    fn reference_str(&self) -> Option<&str> {
        self.reference.as_deref()
    }
}

impl RefStr for IssueArgs {
    fn reference_str(&self) -> Option<&str> {
        self.reference.as_deref()
    }
}

impl RefStr for ResolveArgs {
    fn reference_str(&self) -> Option<&str> {
        Some(self.reference.as_str())
    }
}

const SCENARIO_DATA: &[(SubcommandType, &[&str], &str)] = &[
    (
        SubcommandType::Pr,
        &[
            "VK_CONFIG_PATH",
            "VKCMDS_PR_REFERENCE",
            "VKCMDS_PR_FILES",
            "VKCMDS_PR_SHOW_OUTDATED",
        ],
        "pr",
    ),
    (
        SubcommandType::Issue,
        &["VK_CONFIG_PATH", "VKCMDS_ISSUE_REFERENCE"],
        "issue",
    ),
    (
        SubcommandType::Resolve,
        &[
            "VK_CONFIG_PATH",
            "VKCMDS_RESOLVE_REFERENCE",
            "VKCMDS_RESOLVE_MESSAGE",
        ],
        "resolve",
    ),
];

fn config_for_scenario(scenario: &TestScenario, variant: ConfigVariant) -> String {
    match (scenario.subcommand, variant) {
        (SubcommandType::Pr, ConfigVariant::CliDominant | ConfigVariant::EnvFallback) => format!(
            r#"[cmds.{section}]
reference = "file_ref"
files = ["config.txt"]
show_outdated = false
"#,
            section = scenario.config_section,
        ),
        (SubcommandType::Pr, ConfigVariant::ConfigFallback) => String::new(),
        (SubcommandType::Issue, _) => format!(
            r#"[cmds.{section}]
reference = "file_ref"
"#,
            section = scenario.config_section,
        ),
        (SubcommandType::Resolve, ConfigVariant::CliDominant) => format!(
            r#"[cmds.{section}]
reference = "file_ref"
message = "file message"
"#,
            section = scenario.config_section,
        ),
        (SubcommandType::Resolve, ConfigVariant::EnvFallback | ConfigVariant::ConfigFallback) => {
            format!(
                r#"[cmds.{section}]
message = "file message"
"#,
                section = scenario.config_section,
            )
        }
    }
}

fn create_scenario(subcommand: SubcommandType) -> TestScenario {
    let (kind, env_vars, section) = SCENARIO_DATA
        .iter()
        .find(|(kind, _, _)| *kind == subcommand)
        .copied()
        .unwrap_or_else(|| panic!("scenario data missing for {subcommand:?}"));

    TestScenario {
        subcommand: kind,
        env_vars,
        config_section: section,
    }
}

#[fixture]
fn pr_scenario() -> TestScenario {
    create_scenario(SubcommandType::Pr)
}

#[fixture]
fn issue_scenario() -> TestScenario {
    create_scenario(SubcommandType::Issue)
}

#[fixture]
fn resolve_scenario() -> TestScenario {
    create_scenario(SubcommandType::Resolve)
}

fn assert_reference_equals<T: RefStr>(merged: &T, expected: &str) {
    assert_eq!(merged.reference_str(), Some(expected));
}

#[rstest]
#[case::pr(pr_scenario())]
#[case::issue(issue_scenario())]
#[case::resolve(resolve_scenario())]
#[serial]
fn load_and_merge_prefers_cli_over_other_sources(#[case] scenario: TestScenario) {
    let _guard = EnvGuard::new(scenario.env_vars);

    let cfg = config_for_scenario(&scenario, ConfigVariant::CliDominant);
    let (_config_dir, _config_path) = setup_env_and_config(&cfg);

    match scenario.subcommand {
        SubcommandType::Pr => {
            set_var("VKCMDS_PR_REFERENCE", "env_ref");
            set_var("VKCMDS_PR_FILES", "env.txt");
            set_var("VKCMDS_PR_SHOW_OUTDATED", "false");

            let cli = PrArgs {
                reference: Some(String::from("cli_ref")),
                files: vec![String::from("cli.txt")],
                show_outdated: true,
            };

            let merged = cli.load_and_merge().expect("merge pr args");

            assert_reference_equals(&merged, "cli_ref");
            assert_eq!(merged.files, vec![String::from("cli.txt")]);
            assert!(merged.show_outdated);
        }
        SubcommandType::Issue => {
            set_var("VKCMDS_ISSUE_REFERENCE", "env_ref");

            let cli = IssueArgs {
                reference: Some(String::from("cli_ref")),
            };

            let merged = cli.load_and_merge().expect("merge issue args");

            assert_reference_equals(&merged, "cli_ref");
        }
        SubcommandType::Resolve => {
            set_var("VKCMDS_RESOLVE_REFERENCE", "env_ref");
            set_var("VKCMDS_RESOLVE_MESSAGE", "env message");

            let cli = ResolveArgs {
                reference: String::from("cli_ref"),
                message: Some(String::from("cli message")),
            };

            let merged = cli.load_and_merge().expect("merge resolve args");

            assert_reference_equals(&merged, "cli_ref");
            assert_eq!(merged.message.as_deref(), Some("cli message"));
        }
    }
}

#[rstest]
#[case::pr(pr_scenario())]
#[case::issue(issue_scenario())]
#[case::resolve(resolve_scenario())]
#[serial]
fn load_and_merge_uses_environment_when_cli_defaults(#[case] scenario: TestScenario) {
    let _guard = EnvGuard::new(scenario.env_vars);

    let cfg = config_for_scenario(&scenario, ConfigVariant::EnvFallback);
    let (_config_dir, _config_path) = setup_env_and_config(&cfg);

    match scenario.subcommand {
        SubcommandType::Pr => {
            set_var("VKCMDS_PR_REFERENCE", "env_ref");
            set_var("VKCMDS_PR_FILES", "env_one.rs,env_two.rs");
            set_var("VKCMDS_PR_SHOW_OUTDATED", "1");

            let cli = PrArgs::default();
            let merged = cli.load_and_merge().expect("merge pr args");

            // Only the optional reference can be filled from the environment. Clap
            // initialises vectors and booleans eagerly, so their defaults read as
            // explicit CLI choices and we leave them untouched by config or
            // environment overrides.
            assert_reference_equals(&merged, "env_ref");
            assert!(merged.files.is_empty());
            assert!(!merged.show_outdated);
        }
        SubcommandType::Issue => {
            set_var("VKCMDS_ISSUE_REFERENCE", "env_ref");

            let cli = IssueArgs::default();
            let merged = cli.load_and_merge().expect("merge issue args");

            assert_reference_equals(&merged, "env_ref");
        }
        SubcommandType::Resolve => {
            set_var("VKCMDS_RESOLVE_MESSAGE", "env message");
            set_var("VKCMDS_RESOLVE_REFERENCE", "env_ref");

            let cli = ResolveArgs {
                reference: String::from("cli_ref"),
                message: None,
            };
            let merged = cli.load_and_merge().expect("merge resolve args");

            assert_reference_equals(&merged, "cli_ref");
            assert_eq!(merged.message.as_deref(), Some("env message"));
        }
    }
}

#[rstest]
#[case::pr(pr_scenario())]
#[case::issue(issue_scenario())]
#[case::resolve(resolve_scenario())]
#[serial]
fn load_and_merge_uses_config_or_defaults(#[case] scenario: TestScenario) {
    let _guard = EnvGuard::new(scenario.env_vars);

    let cfg = config_for_scenario(&scenario, ConfigVariant::ConfigFallback);
    let (config_dir, _config_path) = setup_env_and_config(&cfg);

    match scenario.subcommand {
        SubcommandType::Pr => {
            let cli = PrArgs::default();
            let merged = cli.load_and_merge().expect("merge pr args");

            assert!(merged.reference.is_none());
            assert!(!merged.show_outdated);
        }
        SubcommandType::Issue => {
            let cli = IssueArgs::default();

            let _dir = DirGuard::enter(config_dir.path());
            let merged = cli.load_and_merge().expect("merge issue args");
            assert_reference_equals(&merged, "file_ref");
        }
        SubcommandType::Resolve => {
            let cli = ResolveArgs {
                reference: String::from("cli_ref"),
                message: None,
            };

            let _dir = DirGuard::enter(config_dir.path());
            let merged = cli.load_and_merge().expect("merge resolve args");
            assert_reference_equals(&merged, "cli_ref");
            assert_eq!(merged.message.as_deref(), Some("file message"));
        }
    }
}

#[rstest]
#[case::pr(pr_scenario())]
#[case::issue(issue_scenario())]
#[case::resolve(resolve_scenario())]
#[serial]
fn load_and_merge_preserves_cli_instance(#[case] scenario: TestScenario) {
    let _guard = EnvGuard::new(scenario.env_vars);

    match scenario.subcommand {
        SubcommandType::Pr => {
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
            assert_reference_equals(&merged, "cli_ref");
            assert_eq!(merged.files, vec![String::from("cli.txt")]);
            assert!(merged.show_outdated);
        }
        SubcommandType::Issue => {
            let cli = IssueArgs {
                reference: Some(String::from("cli_ref")),
            };
            let snapshot = cli.clone();

            let merged = cli.load_and_merge().expect("merge issue args");

            assert_eq!(cli.reference, snapshot.reference);
            assert_reference_equals(&merged, "cli_ref");
        }
        SubcommandType::Resolve => {
            let cli = ResolveArgs {
                reference: String::from("cli_ref"),
                message: Some(String::from("cli message")),
            };
            let snapshot = cli.clone();

            let merged = cli.load_and_merge().expect("merge resolve args");

            assert_eq!(cli.reference, snapshot.reference);
            assert_eq!(cli.message, snapshot.message);
            assert_reference_equals(&merged, "cli_ref");
            assert_eq!(merged.message.as_deref(), Some("cli message"));
        }
    }
}

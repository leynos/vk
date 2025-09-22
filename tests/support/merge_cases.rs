//! Shared merge precedence fixtures for CLI and subcommand tests.
//!
//! Provides reusable scenario definitions describing how configuration,
//! environment, and CLI inputs interact for each subcommand. Keeping the data
//! here ensures behavioural tests assert the same expectations without
//! duplicating setup logic.

use vk::cli_args::{IssueArgs, PrArgs, ResolveArgs};

type EnvAssignments = &'static [(&'static str, Option<&'static str>)];

#[derive(Copy, Clone)]
struct SubcommandCaseData {
    subcommand: MergeSubcommand,
    scenarios: [(MergeScenario, &'static str, EnvAssignments, bool); 3],
    expectation_builder: fn(MergeScenario) -> MergeExpectation,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MergeSubcommand {
    Pr,
    Issue,
    Resolve,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MergeScenario {
    CliOverEnv,
    EnvOverFile,
    FileOverDefaults,
}

#[derive(Clone, Debug)]
pub enum MergeExpectation {
    Pr {
        cli: PrArgs,
        expected_reference: Option<&'static str>,
        expected_files: &'static [&'static str],
        expected_show_outdated: bool,
    },
    Issue {
        cli: IssueArgs,
        expected_reference: Option<&'static str>,
    },
    Resolve {
        cli: ResolveArgs,
        expected_reference: &'static str,
        expected_message: Option<&'static str>,
    },
}

#[derive(Clone, Debug)]
pub struct MergeCase {
    pub subcommand: MergeSubcommand,
    pub scenario: MergeScenario,
    pub config: &'static str,
    pub env: EnvAssignments,
    pub expectation: MergeExpectation,
    pub enter_config_dir: bool,
}

fn build_cases_from_data(data: &SubcommandCaseData) -> Vec<MergeCase> {
    data.scenarios
        .iter()
        .map(|&(scenario, config, env, enter_config_dir)| MergeCase {
            subcommand: data.subcommand,
            scenario,
            config,
            env,
            expectation: (data.expectation_builder)(scenario),
            enter_config_dir,
        })
        .collect()
}

fn build_pr_expectation(scenario: MergeScenario) -> MergeExpectation {
    match scenario {
        MergeScenario::CliOverEnv => MergeExpectation::Pr {
            cli: build_pr_args(Some("cli_ref"), &["cli.txt"], true),
            expected_reference: Some("cli_ref"),
            expected_files: &["cli.txt"],
            expected_show_outdated: true,
        },
        MergeScenario::EnvOverFile => MergeExpectation::Pr {
            cli: build_pr_args(None, &[], false),
            expected_reference: Some("env_ref"),
            expected_files: &[],
            expected_show_outdated: false,
        },
        MergeScenario::FileOverDefaults => MergeExpectation::Pr {
            cli: build_pr_args(None, &[], false),
            expected_reference: Some("file_ref"),
            expected_files: &[],
            expected_show_outdated: false,
        },
    }
}

fn build_issue_expectation(scenario: MergeScenario) -> MergeExpectation {
    match scenario {
        MergeScenario::CliOverEnv => MergeExpectation::Issue {
            cli: build_issue_args(Some("cli_ref")),
            expected_reference: Some("cli_ref"),
        },
        MergeScenario::EnvOverFile => MergeExpectation::Issue {
            cli: build_issue_args(None),
            expected_reference: Some("env_ref"),
        },
        MergeScenario::FileOverDefaults => MergeExpectation::Issue {
            cli: build_issue_args(None),
            expected_reference: Some("file_ref"),
        },
    }
}

fn build_resolve_expectation(scenario: MergeScenario) -> MergeExpectation {
    match scenario {
        MergeScenario::CliOverEnv => MergeExpectation::Resolve {
            cli: build_resolve_args("cli_ref", Some("cli message")),
            expected_reference: "cli_ref",
            expected_message: Some("cli message"),
        },
        MergeScenario::EnvOverFile => MergeExpectation::Resolve {
            cli: build_resolve_args("cli_ref", None),
            expected_reference: "cli_ref",
            expected_message: Some("env message"),
        },
        MergeScenario::FileOverDefaults => MergeExpectation::Resolve {
            cli: build_resolve_args("cli_ref", None),
            expected_reference: "cli_ref",
            expected_message: Some("file message"),
        },
    }
}

pub fn case(subcommand: MergeSubcommand, scenario: MergeScenario) -> MergeCase {
    all_cases()
        .into_iter()
        .find(|case| case.subcommand == subcommand && case.scenario == scenario)
        .unwrap_or_else(|| panic!("missing merge case for {subcommand:?} and {scenario:?}"))
}

fn all_cases() -> Vec<MergeCase> {
    [
        SubcommandCaseData {
            subcommand: MergeSubcommand::Pr,
            scenarios: [
                (
                    MergeScenario::CliOverEnv,
                    PR_CONFIG,
                    PR_ENV_CLI_OVER_ENV,
                    true,
                ),
                (
                    MergeScenario::EnvOverFile,
                    PR_CONFIG,
                    PR_ENV_ENV_OVER_FILE,
                    true,
                ),
                (
                    MergeScenario::FileOverDefaults,
                    PR_CONFIG,
                    PR_ENV_FILE_OVER_DEFAULTS,
                    true,
                ),
            ],
            expectation_builder: build_pr_expectation,
        },
        SubcommandCaseData {
            subcommand: MergeSubcommand::Issue,
            scenarios: [
                (
                    MergeScenario::CliOverEnv,
                    ISSUE_CONFIG,
                    ISSUE_ENV_CLI_OVER_ENV,
                    false,
                ),
                (
                    MergeScenario::EnvOverFile,
                    ISSUE_CONFIG,
                    ISSUE_ENV_ENV_OVER_FILE,
                    false,
                ),
                (
                    MergeScenario::FileOverDefaults,
                    ISSUE_CONFIG,
                    ISSUE_ENV_FILE_OVER_DEFAULTS,
                    true,
                ),
            ],
            expectation_builder: build_issue_expectation,
        },
        SubcommandCaseData {
            subcommand: MergeSubcommand::Resolve,
            scenarios: [
                (
                    MergeScenario::CliOverEnv,
                    RESOLVE_CONFIG_WITH_REFERENCE,
                    RESOLVE_ENV_CLI_OVER_ENV,
                    false,
                ),
                (
                    MergeScenario::EnvOverFile,
                    RESOLVE_MESSAGE_CONFIG,
                    RESOLVE_ENV_ENV_OVER_FILE,
                    false,
                ),
                (
                    MergeScenario::FileOverDefaults,
                    RESOLVE_MESSAGE_CONFIG,
                    RESOLVE_ENV_FILE_OVER_DEFAULTS,
                    true,
                ),
            ],
            expectation_builder: build_resolve_expectation,
        },
    ]
    .into_iter()
    .flat_map(|data| build_cases_from_data(&data))
    .collect()
}

fn build_pr_args(reference: Option<&str>, files: &[&str], show_outdated: bool) -> PrArgs {
    PrArgs {
        reference: reference.map(str::to_owned),
        files: files.iter().map(|value| String::from(*value)).collect(),
        show_outdated,
    }
}

fn build_issue_args(reference: Option<&str>) -> IssueArgs {
    IssueArgs {
        reference: reference.map(str::to_owned),
    }
}

fn build_resolve_args(reference: &str, message: Option<&str>) -> ResolveArgs {
    ResolveArgs {
        reference: String::from(reference),
        message: message.map(str::to_owned),
    }
}

const PR_CONFIG: &str = r#"[cmds.pr]
reference = "file_ref"
files = ["config.txt"]
show_outdated = false
"#;

const PR_ENV_CLI_OVER_ENV: EnvAssignments = &[
    ("VKCMDS_PR_REFERENCE", Some("env_ref")),
    ("VKCMDS_PR_FILES", Some("env.txt")),
    ("VKCMDS_PR_SHOW_OUTDATED", Some("false")),
];

const PR_ENV_ENV_OVER_FILE: EnvAssignments = &[
    ("VKCMDS_PR_REFERENCE", Some("env_ref")),
    ("VKCMDS_PR_FILES", Some("env_one.rs,env_two.rs")),
    ("VKCMDS_PR_SHOW_OUTDATED", Some("true")),
];

const PR_ENV_FILE_OVER_DEFAULTS: EnvAssignments = &[
    ("VKCMDS_PR_REFERENCE", None),
    ("VKCMDS_PR_FILES", None),
    ("VKCMDS_PR_SHOW_OUTDATED", None),
];

const ISSUE_CONFIG: &str = r#"[cmds.issue]
reference = "file_ref"
"#;

const ISSUE_ENV_CLI_OVER_ENV: EnvAssignments = &[("VKCMDS_ISSUE_REFERENCE", Some("env_ref"))];

const ISSUE_ENV_ENV_OVER_FILE: EnvAssignments = &[("VKCMDS_ISSUE_REFERENCE", Some("env_ref"))];

const ISSUE_ENV_FILE_OVER_DEFAULTS: EnvAssignments = &[("VKCMDS_ISSUE_REFERENCE", None)];

const RESOLVE_CONFIG_WITH_REFERENCE: &str = r#"[cmds.resolve]
reference = "file_ref"
message = "file message"
"#;

const RESOLVE_MESSAGE_CONFIG: &str = r#"[cmds.resolve]
message = "file message"
"#;

const RESOLVE_ENV_CLI_OVER_ENV: EnvAssignments = &[
    ("VKCMDS_RESOLVE_REFERENCE", Some("env_ref")),
    ("VKCMDS_RESOLVE_MESSAGE", Some("env message")),
];

const RESOLVE_ENV_ENV_OVER_FILE: EnvAssignments = &[
    ("VKCMDS_RESOLVE_REFERENCE", None),
    ("VKCMDS_RESOLVE_MESSAGE", Some("env message")),
];

const RESOLVE_ENV_FILE_OVER_DEFAULTS: EnvAssignments = &[
    ("VKCMDS_RESOLVE_REFERENCE", None),
    ("VKCMDS_RESOLVE_MESSAGE", None),
];

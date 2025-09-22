//! Data-driven merge precedence cases consumed by CLI and subcommand tests.
//! Defines subcommands, scenarios, input sources (config/env), and how to set up
//! each case so behavioural tests share a single source of truth.
use super::expectations::{
    MergeExpectation, build_issue_expectation, build_pr_expectation, build_resolve_expectation,
};
use std::sync::LazyLock;

type EnvAssignments = &'static [(&'static str, Option<&'static str>)];

#[derive(Copy, Clone)]
struct SubcommandCaseData {
    subcommand: MergeSubcommand,
    scenarios: [(MergeScenario, &'static str, EnvAssignments, bool); 3],
    expectation_builder: fn(MergeScenario) -> MergeExpectation,
}

/// Subcommands under test for precedence merging.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MergeSubcommand {
    Pr,
    Issue,
    Resolve,
}

/// Precedence scenarios describing which source wins when values conflict.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MergeScenario {
    CliOverEnv,
    EnvOverFile,
    FileOverDefaults,
}

#[derive(Clone, Debug)]
/// Merge scenario fixture consumed by CLI and subcommand tests.
pub struct MergeCase {
    /// Subcommand under test for this precedence scenario.
    pub subcommand: MergeSubcommand,
    /// Concrete combination of CLI, environment, and config sources to evaluate.
    pub scenario: MergeScenario,
    /// Configuration content written to `.vk.toml`.
    pub config: &'static str,
    /// Environment variable assignments applied for the scenario.
    pub env: EnvAssignments,
    /// Expected merge result for the subcommand/scenario pair.
    pub expectation: MergeExpectation,
    /// When true, CLI-harness tests enter the generated config directory before merging.
    ///
    /// This mirrors real CLI execution so relative CLI arguments resolve against the
    /// configuration file’s directory. The subcommand harness always enters the config dir.
    pub enter_config_dir: bool,
}

impl MergeCase {
    /// True when CLI tests must enter the generated config directory before merging.
    pub fn requires_config_dir(&self) -> bool {
        self.enter_config_dir
    }
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

/// Return the unique `MergeCase` for a subcommand/scenario pair.
///
/// Panics if the pair is not defined in `SUBCOMMAND_CASE_DATA`.
pub fn case(subcommand: MergeSubcommand, scenario: MergeScenario) -> MergeCase {
    all_cases()
        .iter()
        .find(|case| case.subcommand == subcommand && case.scenario == scenario)
        .cloned()
        .unwrap_or_else(|| panic!("missing merge case for {subcommand:?} and {scenario:?}"))
}

const SUBCOMMAND_CASE_DATA: [SubcommandCaseData; 3] = [
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
];

fn all_cases() -> &'static [MergeCase] {
    static ALL_CASES: LazyLock<Vec<MergeCase>> = LazyLock::new(|| {
        SUBCOMMAND_CASE_DATA
            .iter()
            .flat_map(build_cases_from_data)
            .collect()
    });

    ALL_CASES.as_slice()
}

const PR_CONFIG: &str = r#"[cmds.pr]
reference = "file_ref"
files = ["config.txt"]
show_outdated = false
"#;

const PR_ENV_CLI_OVER_ENV: EnvAssignments = &[
    ("VKCMDS_PR_REFERENCE", Some("env_ref")),
    ("VKCMDS_PR_FILES", Some(r#"["env.txt"]"#)),
    ("VKCMDS_PR_SHOW_OUTDATED", Some("false")),
];

const PR_ENV_ENV_OVER_FILE: EnvAssignments = &[
    ("VKCMDS_PR_REFERENCE", Some("env_ref")),
    ("VKCMDS_PR_FILES", Some(r#"["env_one.rs","env_two.rs"]"#)),
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

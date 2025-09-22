//! Expected merged outputs for each subcommand/scenario pair used by data-driven tests.
use super::data::MergeScenario;
use vk::cli_args::{IssueArgs, PrArgs, ResolveArgs};

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

pub(super) fn build_pr_expectation(scenario: MergeScenario) -> MergeExpectation {
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
            expected_files: &["env_one.rs", "env_two.rs"],
            expected_show_outdated: true,
        },
        MergeScenario::FileOverDefaults => MergeExpectation::Pr {
            cli: build_pr_args(None, &[], false),
            expected_reference: Some("file_ref"),
            expected_files: &["config.txt"],
            expected_show_outdated: false,
        },
    }
}

pub(super) fn build_issue_expectation(scenario: MergeScenario) -> MergeExpectation {
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

pub(super) fn build_resolve_expectation(scenario: MergeScenario) -> MergeExpectation {
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

//! Tests verifying configuration precedence for subcommands.
//!
//! These tests ensure configuration file values override defaults, are overridden
//! by environment variables, and finally by command-line arguments.

#[path = "support/env.rs"]
mod env_support;
#[path = "support/merge_cases/mod.rs"]
mod merge_cases;
#[path = "support/subcommand.rs"]
mod sub_support;

use merge_cases::{
    MergeCase, MergeExpectation, MergeScenario, MergeSubcommand, case as merge_case,
};
use rstest::rstest;
use serial_test::serial;
use sub_support::merge_with_sources;

fn assert_merge_case(case: MergeCase) {
    // merge_with_sources always enters the config directory; touch the flag so the data helper stays exercised.
    case.requires_config_dir();

    let MergeCase {
        config,
        env,
        expectation,
        ..
    } = case;

    match expectation {
        MergeExpectation::Pr {
            cli,
            expected_reference,
            expected_files,
            expected_show_outdated,
        } => {
            let merged = merge_with_sources(config, env, &cli);
            assert_eq!(merged.reference.as_deref(), expected_reference);
            assert_eq!(
                merged.files,
                expected_files
                    .iter()
                    .map(|value| String::from(*value))
                    .collect::<Vec<_>>(),
            );
            assert_eq!(merged.show_outdated, expected_show_outdated);
        }
        MergeExpectation::Issue {
            cli,
            expected_reference,
        } => {
            let merged = merge_with_sources(config, env, &cli);
            assert_eq!(merged.reference.as_deref(), expected_reference);
        }
        MergeExpectation::Resolve {
            cli,
            expected_reference,
            expected_message,
        } => {
            let merged = merge_with_sources(config, env, &cli);
            assert_eq!(merged.reference, expected_reference);
            assert_eq!(merged.message.as_deref(), expected_message);
        }
    }
}

#[rstest]
#[case(MergeSubcommand::Pr, MergeScenario::CliOverEnv)]
#[case(MergeSubcommand::Pr, MergeScenario::EnvOverFile)]
#[case(MergeSubcommand::Pr, MergeScenario::FileOverDefaults)]
#[case(MergeSubcommand::Issue, MergeScenario::CliOverEnv)]
#[case(MergeSubcommand::Issue, MergeScenario::EnvOverFile)]
#[case(MergeSubcommand::Issue, MergeScenario::FileOverDefaults)]
#[case(MergeSubcommand::Resolve, MergeScenario::CliOverEnv)]
#[case(MergeSubcommand::Resolve, MergeScenario::EnvOverFile)]
#[case(MergeSubcommand::Resolve, MergeScenario::FileOverDefaults)]
#[serial]
fn test_configuration_precedence(
    #[case] subcommand: MergeSubcommand,
    #[case] scenario: MergeScenario,
) {
    let case = merge_case(subcommand, scenario);
    assert_merge_case(case);
}

#[rstest]
#[case::env_overrides_file(MergeScenario::EnvOverFile)]
#[case::cli_overrides_environment(MergeScenario::CliOverEnv)]
#[serial]
fn test_pr_load_and_merge_precedence(#[case] scenario: MergeScenario) {
    let case = merge_case(MergeSubcommand::Pr, scenario);
    assert_merge_case(case);
}

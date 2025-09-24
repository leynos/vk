//! Behavioural coverage for CLI argument merging helpers.

#[path = "support/merge_cases/mod.rs"]
mod merge_cases;
#[path = "support/merge.rs"]
mod merge_support;
#[path = "support/env.rs"]
mod support;

use merge_cases::{
    MergeCase, MergeExpectation, MergeScenario, MergeSubcommand, case as merge_case,
};
use merge_support::{environment_keys, to_owned_vec};
use ortho_config::SubcmdConfigMerge;
use rstest::rstest;
use serial_test::serial;
use support::{EnvGuard, maybe_enter_dir, setup_env_and_config};
use vk::test_utils::{remove_var, set_var};

fn apply_env(assignments: &[(&str, Option<&str>)]) {
    for (key, value) in assignments {
        match value {
            Some(val) => set_var(key, val),
            None => remove_var(key),
        }
    }
}

fn with_case_environment(case: MergeCase, assertions: impl FnOnce(MergeExpectation)) {
    let enter_config_dir = case.requires_config_dir();

    let MergeCase {
        config,
        env,
        expectation,
        ..
    } = case;

    let keys = environment_keys(env);
    let _guard = EnvGuard::new(&keys);
    let (config_dir, _config_path) = setup_env_and_config(config);
    let _dir = maybe_enter_dir(enter_config_dir, config_dir.path());

    apply_env(env);

    assertions(expectation);
}

#[test]
#[serial]
fn env_overrides_false_cli_show_outdated_flag() {
    let case = merge_case(MergeSubcommand::Pr, MergeScenario::EnvOverFile);
    with_case_environment(case, |expectation| match expectation {
        MergeExpectation::Pr {
            cli,
            expected_show_outdated,
            ..
        } => {
            assert!(
                !cli.show_outdated,
                "precondition: CLI defaults should leave show_outdated unset"
            );

            let merged = cli.load_and_merge().expect("merge pr args");
            assert!(
                expected_show_outdated,
                "env_over_file scenario should enable show_outdated"
            );
            assert_eq!(merged.show_outdated, expected_show_outdated);
        }
        other => panic!("expected PR merge expectation, found {other:?}"),
    });
}

fn assert_cli_merge(case: MergeCase) {
    with_case_environment(case, |expectation| match expectation {
        MergeExpectation::Pr {
            cli,
            expected_reference,
            expected_files,
            expected_show_outdated,
        } => {
            let merged = cli.load_and_merge().expect("merge pr args");
            assert_eq!(merged.reference.as_deref(), expected_reference);
            assert_eq!(merged.files, to_owned_vec(expected_files));
            assert_eq!(merged.show_outdated, expected_show_outdated);
        }
        MergeExpectation::Issue {
            cli,
            expected_reference,
        } => {
            let merged = cli.load_and_merge().expect("merge issue args");
            assert_eq!(merged.reference.as_deref(), expected_reference);
        }
        MergeExpectation::Resolve {
            cli,
            expected_reference,
            expected_message,
        } => {
            let merged = cli.load_and_merge().expect("merge resolve args");
            assert_eq!(merged.reference, expected_reference);
            assert_eq!(merged.message.as_deref(), expected_message);
        }
    });
}

fn assert_cli_preserves(case: MergeCase) {
    with_case_environment(case, |expectation| match expectation {
        MergeExpectation::Pr {
            cli,
            expected_reference,
            expected_files,
            expected_show_outdated,
        } => {
            let snapshot = cli.clone();
            let merged = cli.load_and_merge().expect("merge pr args");
            assert_eq!(cli.reference, snapshot.reference);
            assert_eq!(cli.files, snapshot.files);
            assert_eq!(cli.show_outdated, snapshot.show_outdated);
            assert_eq!(merged.reference.as_deref(), expected_reference);
            assert_eq!(merged.files, to_owned_vec(expected_files));
            assert_eq!(merged.show_outdated, expected_show_outdated);
        }
        MergeExpectation::Issue {
            cli,
            expected_reference,
        } => {
            let snapshot = cli.clone();
            let merged = cli.load_and_merge().expect("merge issue args");
            assert_eq!(cli.reference, snapshot.reference);
            assert_eq!(merged.reference.as_deref(), expected_reference);
        }
        MergeExpectation::Resolve {
            cli,
            expected_reference,
            expected_message,
        } => {
            let snapshot = cli.clone();
            let merged = cli.load_and_merge().expect("merge resolve args");
            assert_eq!(cli.reference, snapshot.reference);
            assert_eq!(cli.message, snapshot.message);
            assert_eq!(merged.reference, expected_reference);
            assert_eq!(merged.message.as_deref(), expected_message);
        }
    });
}

#[rstest]
#[case::pr_cli_over_env(merge_case(MergeSubcommand::Pr, MergeScenario::CliOverEnv))]
#[case::pr_env_over_file(merge_case(MergeSubcommand::Pr, MergeScenario::EnvOverFile))]
#[case::pr_file_over_defaults(merge_case(MergeSubcommand::Pr, MergeScenario::FileOverDefaults))]
#[case::issue_cli_over_env(merge_case(MergeSubcommand::Issue, MergeScenario::CliOverEnv))]
#[case::issue_env_over_file(merge_case(MergeSubcommand::Issue, MergeScenario::EnvOverFile))]
#[case::issue_file_over_defaults(merge_case(
    MergeSubcommand::Issue,
    MergeScenario::FileOverDefaults
))]
#[case::resolve_cli_over_env(merge_case(MergeSubcommand::Resolve, MergeScenario::CliOverEnv))]
#[case::resolve_env_over_file(merge_case(MergeSubcommand::Resolve, MergeScenario::EnvOverFile))]
#[case::resolve_file_over_defaults(merge_case(
    MergeSubcommand::Resolve,
    MergeScenario::FileOverDefaults
))]
#[serial]
fn load_and_merge_merges_sources(#[case] case: MergeCase) {
    assert_cli_merge(case);
}

#[rstest]
#[case::pr(merge_case(MergeSubcommand::Pr, MergeScenario::CliOverEnv))]
#[case::issue(merge_case(MergeSubcommand::Issue, MergeScenario::CliOverEnv))]
#[case::resolve(merge_case(MergeSubcommand::Resolve, MergeScenario::CliOverEnv))]
#[serial]
fn load_and_merge_preserves_cli_instance(#[case] case: MergeCase) {
    assert_cli_preserves(case);
}

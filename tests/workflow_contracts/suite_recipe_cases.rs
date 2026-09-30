//! Cases for the feature target's recipe: what the contract accepts as the
//! feature's own tests and what it refuses.
//!
//! These drive `Recipe::faults` with constructed recipes, because a rule
//! checked only against the recipe already in the Makefile passes whether or
//! not it discriminates.

use rstest::rstest;

use super::makefile::{FEATURE, Makefile, Recipe};

/// The recipe this repository deploys.
fn deployed_recipe() -> Recipe {
    Recipe(vec![
        format!(
            "RUSTFLAGS=\"-D warnings\" $(CARGO) test --features {FEATURE} --test resolve $(BUILD_JOBS)"
        ),
        format!(
            "RUSTFLAGS=\"-D warnings\" $(CARGO) test --features {FEATURE} --bin $(APP) resolve::rest $(BUILD_JOBS)"
        ),
    ])
}

#[test]
fn the_deployed_recipe_reports_no_fault() {
    assert_eq!(deployed_recipe().faults(), Vec::<String>::new());
}

#[rstest]
#[case::all_targets(
    "--features unstable-rest-resolve --test resolve",
    "--features unstable-rest-resolve --all-targets --test resolve",
    "widens the run with --all-targets"
)]
#[case::all_features(
    "--features unstable-rest-resolve --test resolve",
    "--all-features --test resolve",
    "widens the run with --all-features"
)]
#[case::workspace(
    "--features unstable-rest-resolve --test resolve",
    "--features unstable-rest-resolve --workspace --test resolve",
    "widens the run with --workspace"
)]
#[case::feature_dropped("--features unstable-rest-resolve --bin", "--bin", "does not enable")]
#[case::integration_test_dropped(
    "--test resolve",
    "--test other",
    "no line runs the `resolve` integration test"
)]
#[case::unit_tests_dropped(
    " resolve::rest",
    " other",
    "no line runs the `resolve::rest` unit tests"
)]
fn a_recipe_that_strays_from_the_features_tests_is_rejected(
    #[case] from: &str,
    #[case] to: &str,
    #[case] fragment: &str,
) {
    let recipe = Recipe(
        deployed_recipe()
            .0
            .iter()
            .map(|line| line.replacen(from, to, 1))
            .collect(),
    );
    let found = recipe.faults();
    assert!(
        found.iter().any(|fault| fault.contains(fragment)),
        "expected {fragment:?}, got {found:?}"
    );
}

/// A recipe that runs something besides the feature's two test groups, or runs
/// them without `cargo test`, is rejected.
#[rstest]
#[case::compile_only_integration(
    "$(CARGO) build --features unstable-rest-resolve --test resolve",
    "is not a `$(CARGO) test` run"
)]
#[case::another_test_target(
    "$(CARGO) test --features unstable-rest-resolve --test graphql_codegen_ui",
    "is not a `$(CARGO) test` run"
)]
#[case::another_unit_filter(
    "$(CARGO) test --features unstable-rest-resolve --bin $(APP) commands::",
    "is not a `$(CARGO) test` run"
)]
#[case::whole_library(
    "$(CARGO) test --features unstable-rest-resolve --lib",
    "is not a `$(CARGO) test` run"
)]
#[case::doctests(
    "$(CARGO) test --features unstable-rest-resolve --doc",
    "is not a `$(CARGO) test` run"
)]
#[case::both_groups_in_one_line(
    "$(CARGO) test --features unstable-rest-resolve --test resolve --bin $(APP) resolve::rest",
    "is not a `$(CARGO) test` run"
)]
fn an_extra_recipe_line_is_rejected(#[case] extra: &str, #[case] fragment: &str) {
    let mut lines = deployed_recipe().0;
    lines.push(extra.to_owned());
    let found = Recipe(lines).faults();
    assert!(
        found.iter().any(|fault| fault.contains(fragment)),
        "expected {fragment:?}, got {found:?}"
    );
}

/// Neither group may be run by a line that only builds it.
#[test]
fn a_build_of_the_integration_test_is_not_a_run_of_it() {
    let recipe = Recipe(vec![
        "$(CARGO) build --features unstable-rest-resolve --test resolve".to_owned(),
        deployed_recipe()
            .0
            .last()
            .expect("the deployed recipe has lines")
            .clone(),
    ]);
    let found = recipe.faults();
    assert!(
        found
            .iter()
            .any(|fault| fault.contains("no line runs the `resolve` integration test")),
        "{found:?}"
    );
}

#[test]
fn a_recipe_is_read_from_its_target_to_the_next_rule() {
    let makefile = "a:\n\tone\n\ttwo\n\nb:\n\tthree\n";
    assert_eq!(
        Makefile(makefile).recipe("a").0,
        vec!["one".to_owned(), "two".to_owned()]
    );
    assert_eq!(Makefile(makefile).recipe("b").0, vec!["three".to_owned()]);
    assert!(Makefile(makefile).recipe("c").0.is_empty());
}

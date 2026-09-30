//! A pull request runs each test once: the suite in the coverage step, and
//! only the tests the `unstable-rest-resolve` feature adds in the feature job.
//!
//! The `unstable-rest-resolve` job used to run `make test`, which is
//! `cargo test --all-targets --all-features`, so every default-feature test
//! ran a second time beside the coverage step. It now runs
//! `make test-unstable-rest-resolve`, whose recipe runs the `resolve`
//! integration test and the `resolve::rest` unit tests: the tests that are
//! compiled out without the feature, and no others.
//!
//! The steps are matched by what they invoke, not by their names, through
//! `suite_shell.rs`, which splits a command the way the shell does: a step
//! named "Test" that runs nothing is not what this rule is about, and a
//! differently named step that runs the suite is. Neither half stands alone.
//! One says no pull-request step runs the suite; the other says the feature
//! job still runs its target and the target's recipe is held to the feature's
//! tests, so satisfying the first by emptying the job, or by widening the
//! recipe back to the suite, is not an option.
//!
//! The sweep reads this repository's own workflows and Makefile; the cases
//! after it drive the rule with constructed text, because a rule
//! parametrized over files that already conform passes whether or not it
//! discriminates.

use cap_std::ambient_authority;
use cap_std::fs_utf8::Dir;
use rstest::rstest;

use crate::reader::{Step, Workflow, WorkflowError, parse_workflow};
use crate::repository;
use makefile::{DefaultGoal, Makefile, Recipe};

#[path = "suite_makefile.rs"]
mod makefile;
#[path = "suite_reader_cases.rs"]
mod reader_cases;
#[path = "suite_shell.rs"]
mod shell;
#[path = "suite_tokenizer.rs"]
mod tokenizer;

/// The action every pull request's suite run goes through.
const COVERAGE_ACTION: &str = "leynos/shared-actions/.github/actions/generate-coverage@";

/// The one target the feature job runs, whose recipe is held to the feature's
/// tests.
const FEATURE_TARGET: &str = "test-unstable-rest-resolve";

/// The feature whose tests the target runs.
const FEATURE: &str = "unstable-rest-resolve";

/// Return the text of this repository's Makefile.
fn repository_makefile() -> Result<String, WorkflowError> {
    let directory = Dir::open_ambient_dir(env!("CARGO_MANIFEST_DIR"), ambient_authority())
        .map_err(|source| WorkflowError::List { source })?;
    directory
        .read_to_string("Makefile")
        .map_err(|source| WorkflowError::Read {
            file: "Makefile".to_owned(),
            source,
        })
}

/// Return every way the feature target's recipe strays from the feature's own
/// tests: a run that drops the feature, one that widens to every target or
/// every feature and so repeats the suite, or a missing half.
fn recipe_faults(recipe: &Recipe) -> Vec<String> {
    let flag = format!("--features {FEATURE}");
    let mut faults: Vec<String> = recipe
        .0
        .iter()
        .flat_map(|line| {
            let mut found = Vec::new();
            if !line.contains(&flag) {
                found.push(format!("`{line}` does not enable {FEATURE}"));
            }
            for widening in ["--all-targets", "--all-features", "--workspace"] {
                if line.contains(widening) {
                    found.push(format!("`{line}` widens the run with {widening}"));
                }
            }
            found
        })
        .collect();
    let runs_integration_test = |line: &String| {
        let words: Vec<&str> = line.split_whitespace().collect();
        words.windows(2).any(|pair| pair == ["--test", "resolve"])
    };
    if !recipe.0.iter().any(runs_integration_test) {
        faults.push("no line runs the `resolve` integration test".to_owned());
    }
    if !recipe
        .0
        .iter()
        .any(|line| line.contains("--bin") && line.contains(" resolve::rest"))
    {
        faults.push("no line runs the `resolve::rest` unit tests".to_owned());
    }
    faults
}

/// Return every step of one workflow.
fn steps_of(workflow: &Workflow) -> impl Iterator<Item = &Step> {
    workflow.jobs.iter().flat_map(|job| &job.steps)
}

/// Return every way one workflow runs the suite outside the coverage step.
fn repeated_runs(workflow: &Workflow, goal: DefaultGoal<'_>) -> Vec<String> {
    workflow
        .jobs
        .iter()
        .flat_map(|job| {
            job.steps
                .iter()
                .filter(|step| {
                    step.run
                        .as_deref()
                        .is_some_and(|run| shell::runs_suite(run, goal.0))
                })
                .map(|step| {
                    format!(
                        "{} step `{}` runs the suite outside coverage",
                        job.coordinate(),
                        step.label()
                    )
                })
        })
        .collect()
}

/// Return whether a step calls the coverage action.
fn is_coverage(step: &Step) -> bool {
    step.uses
        .as_deref()
        .is_some_and(|uses| uses.starts_with(COVERAGE_ACTION))
}

/// Return every way one workflow's coverage steps fail to run every time.
fn coverage_faults(workflow: &Workflow) -> Vec<String> {
    workflow
        .jobs
        .iter()
        .flat_map(|job| job.steps.iter().map(move |step| (job, step)))
        .filter(|(_, step)| is_coverage(step))
        .filter(|(_, step)| step.raw.get("if").is_some())
        .map(|(job, _)| format!("{} coverage step carries an `if:`", job.coordinate()))
        .collect()
}

/// Return every way a workflow fails the contract, named.
fn violations(workflow: &Workflow, goal: DefaultGoal<'_>) -> Vec<String> {
    repeated_runs(workflow, goal)
        .into_iter()
        .chain(coverage_faults(workflow))
        .collect()
}

/// Return whether a command runs `make` on `target`, looking through wrappers.
fn names_target(run: &str, target: &str) -> bool {
    shell::make_targets(run).iter().any(|named| named == target)
}

/// Return whether any `run:` command of the workflow satisfies `wanted`.
fn any_run(workflow: &Workflow, wanted: impl Fn(&str) -> bool) -> bool {
    steps_of(workflow)
        .filter_map(|step| step.run.as_deref())
        .any(wanted)
}

/// Return whether a workflow has a step that runs the feature target.
fn runs_the_feature_target(workflow: &Workflow) -> bool {
    any_run(workflow, |run| names_target(run, FEATURE_TARGET))
}

/// Return whether a workflow has a step that lints the feature.
fn lints_the_feature(workflow: &Workflow) -> bool {
    any_run(workflow, |run| {
        names_target(run, "lint") && run.contains(&format!("--features {FEATURE}"))
    })
}

/// Return the workflows a pull request can start.
fn pull_request_workflows(workflows: &[Workflow]) -> Vec<&Workflow> {
    workflows
        .iter()
        .filter(|workflow| workflow.events.contains("pull_request"))
        .collect()
}

#[rstest]
fn every_pull_request_workflow_runs_the_suite_only_in_coverage(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let workflows = repository?;
    let goal = Makefile(&repository_makefile()?).default_goal();
    let lanes = pull_request_workflows(&workflows);
    assert!(!lanes.is_empty(), "no pull-request workflow was read");
    let faults: Vec<String> = lanes
        .iter()
        .flat_map(|workflow| {
            violations(workflow, DefaultGoal(&goal))
                .into_iter()
                .map(move |fault| format!("{}: {fault}", workflow.file))
        })
        .collect();
    assert!(faults.is_empty(), "{faults:?}");
    let coverage: usize = lanes
        .iter()
        .map(|workflow| steps_of(workflow).filter(|step| is_coverage(step)).count())
        .sum();
    assert_eq!(
        coverage, 1,
        "exactly one pull-request coverage step must run"
    );
    Ok(())
}

#[rstest]
fn the_feature_job_runs_its_target_and_lints_the_feature(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let workflows = repository?;
    let lanes = pull_request_workflows(&workflows);
    assert!(
        lanes
            .iter()
            .any(|workflow| runs_the_feature_target(workflow)),
        "no pull-request step runs `make {FEATURE_TARGET}`"
    );
    assert!(
        lanes.iter().any(|workflow| lints_the_feature(workflow)),
        "no pull-request step lints `{FEATURE}`"
    );
    Ok(())
}

#[test]
fn the_feature_target_runs_only_the_features_own_tests() -> Result<(), WorkflowError> {
    let recipe = Makefile(&repository_makefile()?).recipe(FEATURE_TARGET);
    assert!(
        !recipe.0.is_empty(),
        "the Makefile has no `{FEATURE_TARGET}` recipe"
    );
    assert_eq!(recipe_faults(&recipe), Vec::<String>::new());
    Ok(())
}

/// A pull-request workflow written the way this repository deploys it, with
/// `extra` added as a step of the feature job.
fn conforming(extra: &str) -> Result<Workflow, WorkflowError> {
    parse_workflow(
        "coverage.yml",
        &format!(
            concat!(
                "on:\n  pull_request:\n    branches: [main]\njobs:\n",
                "  build-test:\n    runs-on: ubuntu-latest\n    steps:\n",
                "      - name: Lint\n        run: make lint\n",
                "      - name: Generate coverage\n",
                "        uses: leynos/shared-actions/.github/actions/generate-coverage@abc\n",
                "        with:\n          output-path: lcov.info\n",
                "  unstable-rest-resolve:\n    runs-on: ubuntu-latest\n    steps:\n",
                "      - name: Lint (feature-gated)\n",
                "        run: make lint CLIPPY_FLAGS=\"--features unstable-rest-resolve\"\n",
                "      - name: Test unstable REST resolve (feature-gated)\n",
                "        run: CARGO=\"cargo --locked\" make test-unstable-rest-resolve\n",
                "{extra}",
            ),
            extra = extra,
        ),
    )
}

/// A goal for the constructed workflows: a bare `make` builds.
const BUILD_GOAL: &str = "build";

#[test]
fn the_deployed_shape_reports_no_violation() -> Result<(), WorkflowError> {
    let workflow = conforming("")?;
    assert_eq!(
        violations(&workflow, DefaultGoal(BUILD_GOAL)),
        Vec::<String>::new()
    );
    assert!(runs_the_feature_target(&workflow));
    assert!(lints_the_feature(&workflow));
    Ok(())
}

#[rstest]
#[case::make_test("      - name: Test\n        run: make test\n")]
#[case::make_all("      - name: All\n        run: make all\n")]
#[case::nextest("      - name: Nextest\n        run: cargo nextest run\n")]
#[case::cargo_test(
    "      - name: Test\n        run: RUSTFLAGS='-D warnings' cargo test --all-targets --all-features\n"
)]
#[case::login_shell("      - name: Shell\n        run: bash -lc 'cargo test'\n")]
#[case::run_block("      - name: Block\n        run: |\n          set -eu\n          make test\n")]
#[case::wrapped("      - name: Wrapped\n        run: timeout 20m make test\n")]
fn a_second_suite_run_is_rejected(#[case] step: &str) -> Result<(), WorkflowError> {
    let found = violations(&conforming(step)?, DefaultGoal(BUILD_GOAL));
    assert!(
        found
            .iter()
            .any(|fault| fault.contains("runs the suite outside coverage")),
        "{found:?}"
    );
    Ok(())
}

#[rstest]
#[case::lint("      - name: Lint again\n        run: make lint\n")]
#[case::contracts("      - name: Contracts\n        run: make test-workflow-contracts\n")]
#[case::build("      - name: Build\n        run: cargo build --all-targets\n")]
#[case::echo("      - name: Say\n        run: echo make test\n")]
#[case::dry_run("      - name: Dry\n        run: make -n test\n")]
#[case::bare_make("      - name: Bare\n        run: make\n")]
#[case::feature_target_again(
    "      - name: Again\n        run: CARGO=\"cargo --locked\" make test-unstable-rest-resolve\n"
)]
fn a_harmless_step_is_accepted(#[case] step: &str) -> Result<(), WorkflowError> {
    let found = violations(&conforming(step)?, DefaultGoal(BUILD_GOAL));
    assert_eq!(found, Vec::<String>::new());
    Ok(())
}

#[test]
fn a_guarded_coverage_step_is_rejected() -> Result<(), WorkflowError> {
    let workflow = parse_workflow(
        "coverage.yml",
        concat!(
            "on: pull_request\njobs:\n  a:\n    steps:\n",
            "      - uses: leynos/shared-actions/.github/actions/generate-coverage@abc\n",
            "        if: github.event_name == 'push'\n",
        ),
    )?;
    assert_eq!(
        violations(&workflow, DefaultGoal(BUILD_GOAL)),
        vec!["coverage.yml:a coverage step carries an `if:`".to_owned()]
    );
    Ok(())
}

#[rstest]
#[case::echoed("      - name: Say\n        run: echo make test-unstable-rest-resolve\n")]
#[case::other_target("      - name: Other\n        run: make test-scripts\n")]
fn a_step_that_does_not_run_the_feature_target_does_not_count(
    #[case] step: &str,
) -> Result<(), WorkflowError> {
    let workflow = parse_workflow(
        "coverage.yml",
        &format!("on: pull_request\njobs:\n  a:\n    steps:\n{step}"),
    )?;
    assert!(!runs_the_feature_target(&workflow));
    Ok(())
}

#[test]
fn a_bare_make_is_a_suite_run_where_the_default_goal_runs_the_suite() -> Result<(), WorkflowError> {
    let step = "      - name: Bare\n        run: make\n";
    let found = violations(&conforming(step)?, DefaultGoal("all"));
    assert!(
        found.iter().any(|fault| fault.contains("outside coverage")),
        "{found:?}"
    );
    Ok(())
}

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
    assert_eq!(recipe_faults(&deployed_recipe()), Vec::<String>::new());
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
    let found = recipe_faults(&recipe);
    assert!(
        found.iter().any(|fault| fault.contains(fragment)),
        "expected {fragment:?}, got {found:?}"
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

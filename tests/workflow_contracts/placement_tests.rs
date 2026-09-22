//! The placement rules and the runner reader, tested against constructed
//! workflows and generated input.
//!
//! Run against the repository alone, a placement rule is only ever seen
//! passing. These cases break each rule one way at a time, and the properties
//! vary the inputs a maintainer writes by hand: runner expressions, runner
//! forms, ceilings and registries.

use std::collections::BTreeSet;

use proptest::prelude::*;
use rstest::rstest;

use crate::placement::{CEILING_BOUNDS, ceiling_fault, fork_fallback_fault, trunk_faults};
use crate::reader::{Job, RunnerSelection, Workflow, WorkflowError, arms_of, parse_workflow};
use crate::registry::labels_in_use;

/// The fork fallback every pull-request lane carries.
const FALLBACK: &str =
    "${{ github.event.pull_request.head.repo.fork && 'ubuntu-latest' || 'ubicloud-standard-2' }}";

/// Return a workflow answering `on` whose one job, `id`, declares `body`.
fn workflow(on: &str, id: &str, body: &str) -> Result<Workflow, WorkflowError> {
    parse_workflow("x.yml", &format!("on: {on}\njobs:\n  {id}:\n{body}"))
}

/// Return the one job of a workflow built by [`workflow`].
fn only_job(workflow: &Workflow) -> &Job {
    workflow.jobs.first().expect("the job is parsed")
}

#[rstest]
#[case::literal("    runs-on: ubuntu-latest\n", RunnerSelection::Literal("ubuntu-latest".into()))]
#[case::sequence(
    "    runs-on: [self-hosted, paid]\n",
    RunnerSelection::Labels(vec!["self-hosted".into(), "paid".into()])
)]
#[case::group(
    "    runs-on: {group: g, labels: paid}\n",
    RunnerSelection::Group { group: Some("g".into()), labels: vec!["paid".into()] }
)]
#[case::delegated("    uses: ./.github/workflows/y.yml\n", RunnerSelection::Delegated)]
fn every_runner_form_is_read(
    #[case] body: &str,
    #[case] expected: RunnerSelection,
) -> Result<(), WorkflowError> {
    assert_eq!(only_job(&workflow("push", "j", body)?).runs_on, expected);
    Ok(())
}

#[test]
fn a_runs_on_of_no_accepted_shape_is_an_error() {
    assert!(matches!(
        workflow("push", "j", "    runs-on: 3\n"),
        Err(WorkflowError::RunsOn { .. })
    ));
}

#[test]
fn a_paid_label_in_a_sequence_is_in_use() -> Result<(), WorkflowError> {
    let workflows = [workflow("push", "j", "    runs-on: [self-hosted, paid]\n")?];
    assert_eq!(
        labels_in_use(&workflows),
        BTreeSet::from(["self-hosted".to_owned(), "paid".to_owned()])
    );
    Ok(())
}

#[rstest]
#[case::fallback(FALLBACK, true)]
#[case::bare_label("ubicloud-standard-2", false)]
#[case::wrong_field(
    "${{ github.event_name == 'push' && 'ubuntu-latest' || 'ubicloud-standard-2' }}",
    false
)]
#[case::wrong_arm(
    "${{ github.event.pull_request.head.repo.fork && 'ubuntu-latest' || 'other' }}",
    false
)]
fn a_pull_request_lane_must_fall_back_for_a_fork(
    #[case] runs_on: &str,
    #[case] is_valid: bool,
) -> Result<(), WorkflowError> {
    let lane = workflow(
        "pull_request",
        "j",
        &format!("    runs-on: \"{runs_on}\"\n"),
    )?;
    assert_eq!(fork_fallback_fault(only_job(&lane)).is_none(), is_valid);
    Ok(())
}

#[test]
fn a_trunk_lane_reverted_to_a_hosted_runner_is_refused() -> Result<(), WorkflowError> {
    let hosted = [workflow("push", "build", "    runs-on: ubuntu-latest\n")?];
    let paid = [workflow(
        "push",
        "build",
        "    runs-on: ubicloud-standard-2\n",
    )?];
    assert_eq!(trunk_faults(&hosted).1.len(), 1);
    assert_eq!(trunk_faults(&paid), (1, Vec::new()));
    Ok(())
}

#[test]
fn a_paid_lane_without_measured_bounds_is_refused() -> Result<(), WorkflowError> {
    let unknown = workflow(
        "push",
        "novel",
        "    runs-on: ubicloud-standard-2\n    timeout-minutes: 10\n",
    )?;
    let unbounded = workflow("push", "build", "    runs-on: ubicloud-standard-2\n")?;
    assert!(ceiling_fault(only_job(&unknown)).is_some());
    assert!(ceiling_fault(only_job(&unbounded)).is_some());
    Ok(())
}

/// A label a workflow might plausibly name, quote characters excluded.
fn label() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,20}"
}

/// Return each label single-quoted, so a generated `null` or `true` stays a
/// label rather than becoming a YAML keyword the reader rightly skips.
fn quoted(labels: &[String]) -> Vec<String> {
    labels.iter().map(|one| format!("'{one}'")).collect()
}

/// Generate one to three labels.
fn labels() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(label(), 1..4)
}

/// Generate one `runs-on` value in any accepted form, with the labels it
/// should be read as reaching.
fn selection() -> impl Strategy<Value = (String, Vec<String>)> {
    prop_oneof![
        label().prop_map(|one| (format!("'{one}'"), vec![one])),
        labels().prop_map(|many| (format!("[{}]", quoted(&many).join(", ")), many)),
        labels().prop_map(|many| {
            (
                format!("{{group: g, labels: [{}]}}", quoted(&many).join(", ")),
                many,
            )
        }),
        labels().prop_map(|many| {
            (
                format!("\"${{{{ x && {} }}}}\"", quoted(&many).join(" || ")),
                many,
            )
        }),
    ]
}

proptest! {
    #[test]
    fn arms_are_exactly_the_quoted_runs(
        arms in prop::collection::vec(label(), 0..5),
        gap in "[ \n\t]{0,4}",
    ) {
        let expression = arms
            .iter()
            .map(|arm| format!("{gap}'{arm}'{gap}"))
            .collect::<Vec<_>>()
            .join("||");
        prop_assert_eq!(arms_of(&expression), arms);
    }

    #[test]
    fn a_dangling_quote_contributes_no_arm(
        complete in prop::collection::vec(label(), 1..4),
        dangling in label(),
    ) {
        let closed = complete.iter().map(|arm| format!("'{arm}'")).collect::<Vec<_>>().join(" || ");
        prop_assert_eq!(arms_of(&format!("{closed} || '{dangling}")), complete);
    }

    #[test]
    fn labels_in_use_match_a_reference_model(
        declared in prop::collection::vec((selection(), any::<bool>()), 1..5),
    ) {
        // Each job either declares the generated runner or delegates; the
        // model is every declared label that is not a frozen hosted one.
        let body: String = declared
            .iter()
            .enumerate()
            .map(|(index, ((runs_on, _), is_delegated))| if *is_delegated {
                format!("  j{index}:\n    uses: ./.github/workflows/y.yml\n")
            } else {
                format!("  j{index}:\n    runs-on: {runs_on}\n")
            })
            .collect();
        let parsed = parse_workflow("x.yml", &format!("on: push\njobs:\n{body}"))
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        let model: BTreeSet<String> = declared
            .iter()
            .filter(|(_, is_delegated)| !is_delegated)
            .flat_map(|((_, labels), _)| labels.clone())
            .filter(|label| !crate::registry::HOSTED_LABELS.contains(&label.as_str()))
            .collect();
        prop_assert_eq!(labels_in_use(&[parsed]), model);
    }

    #[test]
    fn a_ceiling_passes_exactly_within_its_measured_bounds(
        lane in 0_usize..CEILING_BOUNDS.len(),
        minutes in 0_u64..400,
    ) {
        let (id, lowest, highest) = CEILING_BOUNDS.get(lane).copied().expect("the index is in range");
        let body = format!("    runs-on: ubicloud-standard-2\n    timeout-minutes: {minutes}\n");
        let parsed = workflow("push", id, &body).map_err(|err| TestCaseError::fail(err.to_string()))?;
        let is_within = (lowest..=highest).contains(&minutes);
        prop_assert_eq!(ceiling_fault(only_job(&parsed)).is_none(), is_within);
    }
}

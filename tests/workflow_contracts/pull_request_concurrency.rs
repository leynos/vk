//! Every pull-request-startable workflow cancels superseded runs.
//!
//! Pushing twice to a pull request in quick succession leaves the first run
//! charging minutes for a result nobody will read. GitHub cancels it only when
//! the workflow declares a concurrency group and asks for it, so the fact is a
//! property of every workflow a pull request can start, not of any one job.
//!
//! The sweep reads this repository's own workflows; the cases after it drive
//! the rule with constructed workflows, because a rule parametrized over files
//! that already conform passes whether or not it discriminates. Those cases
//! are what prove it rejects a deleted line, a literal `true` that would also
//! cancel a push to `main`, and a group keyed on the run identifier that
//! serializes nothing.

use rstest::rstest;
use serde_norway::Value;

use crate::reader::{Workflow, WorkflowError, parse_workflow};
use crate::repository;

/// The trigger a pull request starts.
///
/// `pull_request_target` is deliberately excluded. It runs with the base
/// repository's token, and the workflow on it here merges; cancelling that
/// mid-write is not a saving.
const PULL_REQUEST_TRIGGER: &str = "pull_request";

/// The only accepted `cancel-in-progress` value.
///
/// A literal `true` would cancel a push to `main` or a dispatch sharing the
/// group, so the contract requires the guarded expression rather than merely a
/// truthy setting.
const CANCEL_IN_PROGRESS: &str = "${{ github.event_name == 'pull_request' }}";

/// The only accepted concurrency group.
///
/// Each part carries identity. Without the workflow name, two workflows
/// would share a group and cancel each other; without the pull request
/// number, every pull request would share one and a push to one would cancel
/// another's run; and `github.ref` is the fallback for an event with no pull
/// request. A group keyed on `github.run_id` is unique per run, so it
/// serializes nothing and never cancels a predecessor.
const CONCURRENCY_GROUP: &str =
    "${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}";

/// Return a concurrency setting as text, whether written as text or not.
fn text_of(setting: Option<&Value>) -> Option<String> {
    match setting? {
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        other => Some(format!("{other:?}")),
    }
}

/// Return why a concurrency group is not the accepted one, if it is not.
fn group_violation(group: Option<&str>) -> Option<String> {
    match group {
        None | Some("") => Some("declares no concurrency group".to_owned()),
        Some(CONCURRENCY_GROUP) => None,
        Some(other) => Some(format!(
            "sets its concurrency group to {other} and not {CONCURRENCY_GROUP}"
        )),
    }
}

/// Return why a `cancel-in-progress` value is not the accepted one, if it is
/// not.
fn cancel_violation(value: Option<&str>) -> Option<String> {
    match value {
        None => Some("sets no cancel-in-progress".to_owned()),
        Some(CANCEL_IN_PROGRESS) => None,
        Some(other) => Some(format!(
            "sets cancel-in-progress to {other} and not {CANCEL_IN_PROGRESS}"
        )),
    }
}

/// Return every way a workflow's concurrency fails the contract, named.
fn violations(workflow: &Workflow) -> Vec<String> {
    let Some(block) = workflow.raw.get("concurrency") else {
        return vec!["declares no concurrency: block".to_owned()];
    };
    let group = match block {
        Value::String(name) => Some(name.clone()),
        _ => text_of(block.get("group")),
    };
    let cancel = text_of(block.get("cancel-in-progress"));
    group_violation(group.as_deref())
        .into_iter()
        .chain(cancel_violation(cancel.as_deref()))
        .collect()
}

/// Return the workflows a pull request can start.
fn pull_request_workflows(workflows: &[Workflow]) -> Vec<&Workflow> {
    workflows
        .iter()
        .filter(|workflow| workflow.events.contains(PULL_REQUEST_TRIGGER))
        .collect()
}

#[rstest]
fn every_pull_request_workflow_cancels_superseded_runs(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let workflows = repository?;
    let lanes = pull_request_workflows(&workflows);
    assert!(
        !lanes.is_empty(),
        "no workflow was read as pull-request-startable, so this sweep \
         asserts nothing"
    );
    let faults: Vec<String> = lanes
        .iter()
        .flat_map(|workflow| {
            violations(workflow)
                .into_iter()
                .map(move |fault| format!("{} {fault}", workflow.file))
        })
        .collect();
    assert!(faults.is_empty(), "{faults:?}");
    Ok(())
}

/// Return a workflow written the way this repository deploys it, with
/// `concurrency` replaced by `block`.
fn conforming(block: &str) -> Result<Workflow, WorkflowError> {
    parse_workflow(
        "ci.yml",
        &format!("name: CI\non:\n  pull_request:\n    branches: [main]\n{block}\njobs: {{}}\n"),
    )
}

/// The concurrency block this repository deploys.
const DEPLOYED: &str = concat!(
    "concurrency:\n",
    "  group: ${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}\n",
    "  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n",
);

/// The deployed block with its guarded expression replaced by a literal.
const LITERAL_TRUE: &str = concat!(
    "concurrency:\n",
    "  group: ${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}\n",
    "  cancel-in-progress: true\n",
);

/// The deployed block with the literal quoted, which YAML reads as text.
const QUOTED_TRUE: &str = concat!(
    "concurrency:\n",
    "  group: ${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}\n",
    "  cancel-in-progress: 'true'\n",
);

#[test]
fn the_deployed_shape_reports_no_violation() -> Result<(), WorkflowError> {
    let workflow = conforming(DEPLOYED)?;
    assert_eq!(violations(&workflow), Vec::<String>::new());
    Ok(())
}

#[rstest]
#[case::cancel_line_removed(
    "concurrency:\n  group: ${{ github.workflow }}\n",
    "sets no cancel-in-progress"
)]
#[case::literal_true(LITERAL_TRUE, "cancel-in-progress to true")]
#[case::quoted_true(QUOTED_TRUE, "cancel-in-progress to true")]
#[case::run_id_group(
    "concurrency:\n  group: ${{ github.run_id }}\n  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n",
    "github.run_id"
)]
#[case::no_workflow_name(
    "concurrency:\n  group: ${{ github.event.pull_request.number || github.ref }}\n  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n",
    "concurrency group to"
)]
#[case::no_pull_request_number(
    "concurrency:\n  group: ${{ github.workflow }}-${{ github.ref }}\n  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n",
    "concurrency group to"
)]
#[case::no_ref_fallback(
    "concurrency:\n  group: ${{ github.workflow }}-${{ github.event.pull_request.number }}\n  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n",
    "concurrency group to"
)]
#[case::fixed_group(
    "concurrency:\n  group: ci\n  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n",
    "concurrency group to"
)]
#[case::scalar_group("concurrency: ci\n", "sets no cancel-in-progress")]
#[case::no_concurrency_block("", "no concurrency")]
fn a_non_conforming_workflow_is_rejected(
    #[case] block: &str,
    #[case] fragment: &str,
) -> Result<(), WorkflowError> {
    let found = violations(&conforming(block)?);
    assert!(
        found.iter().any(|fault| fault.contains(fragment)),
        "expected a violation mentioning {fragment:?}, got {found:?}"
    );
    Ok(())
}

#[rstest]
#[case::push_only("on:\n  push:\n    branches: [main]\n")]
#[case::target("on:\n  pull_request_target:\n    types: [opened]\n")]
#[case::target_inline("on: [pull_request_target]\n")]
#[case::schedule("on:\n  schedule:\n    - cron: '0 3 * * *'\n")]
fn a_workflow_no_pull_request_starts_is_out_of_scope(
    #[case] text: &str,
) -> Result<(), WorkflowError> {
    let workflows = [parse_workflow("x.yml", text)?];
    assert!(pull_request_workflows(&workflows).is_empty());
    Ok(())
}

#[rstest]
#[case::mapping("on:\n  push:\n  pull_request:\n")]
#[case::quoted_key("'on':\n  pull_request:\n    types: [opened]\n")]
#[case::sequence("on: [push, pull_request]\n")]
#[case::bare_scalar("on: pull_request\n")]
fn every_trigger_shape_starts_a_pull_request_lane(#[case] text: &str) -> Result<(), WorkflowError> {
    let workflows = [parse_workflow("x.yml", text)?];
    assert_eq!(pull_request_workflows(&workflows).len(), 1);
    Ok(())
}

#[test]
fn a_concurrency_block_under_a_job_does_not_count() -> Result<(), WorkflowError> {
    // Only the workflow-level block cancels a superseded run of the whole
    // workflow; one written under a job must not satisfy the contract.
    let text = concat!(
        "on: pull_request\n",
        "jobs:\n  a:\n    concurrency:\n      group: g\n",
        "      cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n",
    );
    let found = violations(&parse_workflow("x.yml", text)?);
    assert_eq!(found, vec!["declares no concurrency: block".to_owned()]);
    Ok(())
}

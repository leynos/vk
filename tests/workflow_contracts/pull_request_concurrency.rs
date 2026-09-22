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

/// A group keyed on the run identifier is unique per run, so it serializes
/// nothing and can never cancel a predecessor.
const RUN_ID_EXPRESSION: &str = "github.run_id";

/// Return a concurrency setting as text, whether written as text or not.
fn text_of(setting: Option<&Value>) -> Option<String> {
    match setting? {
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        other => Some(format!("{other:?}")),
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
    let group_fault = match group.as_deref() {
        None | Some("") => Some("declares no concurrency group".to_owned()),
        Some(text) if text.contains(RUN_ID_EXPRESSION) => {
            Some(format!("keys its concurrency group on {RUN_ID_EXPRESSION}"))
        }
        Some(_) => None,
    };
    let cancel_fault = match text_of(block.get("cancel-in-progress")).as_deref() {
        None => Some("sets no cancel-in-progress".to_owned()),
        Some(CANCEL_IN_PROGRESS) => None,
        Some(value) => Some(format!(
            "sets cancel-in-progress to {value} and not {CANCEL_IN_PROGRESS}"
        )),
    };
    group_fault.into_iter().chain(cancel_fault).collect()
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
    "  group: ${{ github.workflow }}-${{ github.event.pull_request.number }}\n",
    "  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n",
);

/// The deployed block with its guarded expression replaced by a literal.
const LITERAL_TRUE: &str = concat!(
    "concurrency:\n",
    "  group: ${{ github.workflow }}-${{ github.event.pull_request.number }}\n",
    "  cancel-in-progress: true\n",
);

/// The deployed block with the literal quoted, which YAML reads as text.
const QUOTED_TRUE: &str = concat!(
    "concurrency:\n",
    "  group: ${{ github.workflow }}-${{ github.event.pull_request.number }}\n",
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

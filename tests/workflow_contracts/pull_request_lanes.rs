//! What a pull-request lane may not do, and what it must.
//!
//! A pull-request lane must not reach CodeScene at all: the upload is refused
//! for a branch CodeScene does not analyse, and the token that authenticates
//! it has no business on the fork-facing side of the repository. What replaces
//! the changed-line gate is the coverage ratchet, which every lane generating
//! coverage must run.
//!
//! Both rules run over the pull-request closure rather than over the
//! workflows whose own triggers name a pull request, so a reusable workflow
//! called from a lane is held to the lane's rules.

use rstest::rstest;

use crate::repository;

use crate::codescene::{CODESCENE_TOKEN, TOKEN_SECRET, calls_codescene, holds_token, operation_of};
use crate::reader::{Job, Step, Workflow, WorkflowError, pull_request_closure};

/// Return every way one step reaches CodeScene or its token.
fn step_faults(job: &Job, step: &Step) -> Vec<String> {
    let at = || format!("{} step {:?}", job.coordinate(), step.label());
    let contact = operation_of(step).map(|_| format!("{} contacts CodeScene", at()));
    let token = holds_token(step).then(|| format!("{} holds {CODESCENE_TOKEN}", at()));
    contact.into_iter().chain(token).collect()
}

/// Return every way one job reaches CodeScene or its token, steps included.
fn job_faults(job: &Job) -> Vec<String> {
    let coordinate = job.coordinate();
    let checks = [
        (calls_codescene(job), "calls a CodeScene workflow"),
        (
            job.declares_env(CODESCENE_TOKEN),
            "exports the token into every step",
        ),
        (
            !job.secret_sites(TOKEN_SECRET).is_empty(),
            "reads the token outside its steps",
        ),
        (
            job.inherits_secrets,
            "forwards every secret, the token among them, with `secrets: inherit`",
        ),
    ];
    checks
        .into_iter()
        .filter(|(is_fault, _)| *is_fault)
        .map(|(_, why)| format!("{coordinate} {why}"))
        .chain(job.steps.iter().flat_map(|step| step_faults(job, step)))
        .collect()
}

/// Return every way one workflow's own scope reaches the token.
fn workflow_faults(workflow: &Workflow) -> Vec<String> {
    let exported =
        workflow.declares_env(CODESCENE_TOKEN) || !workflow.secret_sites(TOKEN_SECRET).is_empty();
    exported
        .then(|| {
            format!(
                "{} reads the token at workflow level, reaching every step",
                workflow.file
            )
        })
        .into_iter()
        .collect()
}

/// Return every way a workflow running for a pull request reaches CodeScene.
///
/// # Errors
///
/// Returns [`WorkflowError::UnresolvedCall`] when a lane calls a local
/// workflow that does not exist.
pub(crate) fn reach_faults(workflows: &[Workflow]) -> Result<Vec<String>, WorkflowError> {
    Ok(pull_request_closure(workflows)?
        .into_iter()
        .flat_map(|workflow| {
            workflow_faults(workflow)
                .into_iter()
                .chain(workflow.jobs.iter().flat_map(job_faults))
        })
        .collect())
}

/// Return every coverage-generating step of one workflow, with its job.
pub(crate) fn generators_of(workflow: &Workflow) -> Vec<(&Job, &Step)> {
    workflow
        .jobs
        .iter()
        .flat_map(|job| job.steps.iter().map(move |step| (job, step)))
        .filter(|(_, step)| {
            step.uses
                .as_deref()
                .is_some_and(|action| action.contains("generate-coverage"))
        })
        .collect()
}

/// Return every pull-request coverage lane, and each generator that does not
/// ratchet.
///
/// A coverage lane is a workflow in the closure that generates coverage,
/// derived rather than named: requiring every pull-request workflow to
/// generate coverage would refuse `dependabot-automerge.yml`, which rightly
/// generates none. Each lane is judged on its own generators, since pooling
/// them lets one lane's ratchet stand in for another's.
///
/// # Errors
///
/// Returns [`WorkflowError::UnresolvedCall`] when a lane calls a local
/// workflow that does not exist.
pub(crate) fn ratchet_faults(
    workflows: &[Workflow],
) -> Result<(usize, Vec<String>), WorkflowError> {
    let lanes: Vec<&Workflow> = pull_request_closure(workflows)?
        .into_iter()
        .filter(|workflow| !generators_of(workflow).is_empty())
        .collect();
    let faults = lanes
        .iter()
        .flat_map(|workflow| generators_of(workflow))
        .filter(|(_, step)| step.input("with-ratchet") != Some("true"))
        .map(|(job, step)| {
            format!(
                "{} step {:?} passes with-ratchet {:?}",
                job.coordinate(),
                step.label(),
                step.input("with-ratchet")
            )
        })
        .collect();
    Ok((lanes.len(), faults))
}

#[rstest]
fn no_pull_request_lane_contacts_codescene(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let workflows = repository?;
    assert!(
        !pull_request_closure(&workflows)?.is_empty(),
        "the repository must run at least one pull-request lane, or every \
         assertion here passes over an empty set"
    );
    let faults = reach_faults(&workflows)?;
    assert!(
        faults.is_empty(),
        "a pull-request lane must not reach CodeScene or its token: {faults:?}"
    );
    Ok(())
}

#[rstest]
fn every_pull_request_workflow_ratchets_its_own_coverage(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let (lanes, faults) = ratchet_faults(&repository?)?;
    assert!(
        lanes > 0,
        "a pull-request lane must generate coverage, or the ratchet has no \
         subject and this contract asserts nothing"
    );
    assert!(
        faults.is_empty(),
        "a lane generating coverage without the ratchet measures nothing it \
         can fail on, and the ratchet is the whole of what replaces the \
         CodeScene changed-line gate: {faults:?}"
    );
    Ok(())
}

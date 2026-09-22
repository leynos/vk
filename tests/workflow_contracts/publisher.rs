//! Who owns the coverage upload, and how it is held to `main`.
//!
//! The push-to-main workflow is the only one that may reach CodeScene, it
//! uploads rather than gates, and the token lives on its upload step and
//! nowhere else. The upload step is guarded twice, on the token and on the
//! ref, because the publisher also answers `workflow_dispatch`, which runs
//! against any branch. And the publisher's runs queue rather than cancel, so
//! a later push cannot abandon an earlier upload and its baseline.

use rstest::rstest;

use crate::repository;

use crate::codescene::{
    CODESCENE_TOKEN, Operation, REF_GUARD, TOKEN_GUARD, TOKEN_INPUT, TOKEN_SECRET, TOKEN_SITE,
    calls_codescene, holds_token, is_publisher, operation_of,
};
use crate::reader::{Condition, Job, Step, Workflow, WorkflowError, jobs, steps};

/// Return every step that reaches CodeScene, with what it asks.
fn contacts(workflows: &[Workflow]) -> Vec<(&Workflow, &Job, &Step, Operation)> {
    steps(workflows)
        .into_iter()
        .filter_map(|(workflow, job, step)| {
            operation_of(step).map(|operation| (workflow, job, step, operation))
        })
        .collect()
}

/// Return every step or job reaching CodeScene outside the publisher.
pub(crate) fn placement_faults(workflows: &[Workflow]) -> Vec<String> {
    let by_step = contacts(workflows)
        .into_iter()
        .filter(|(workflow, ..)| !is_publisher(workflow))
        .map(|(workflow, job, step, _)| {
            format!(
                "{} step {:?} reaches CodeScene, but {} answers {:?} rather than a \
                 push to the published branch alone",
                job.coordinate(),
                step.label(),
                workflow.file,
                workflow.events
            )
        });
    let by_call = jobs(workflows)
        .into_iter()
        .filter(|(workflow, job)| !is_publisher(workflow) && calls_codescene(job))
        .map(|(_, job)| format!("{} calls a CodeScene workflow", job.coordinate()));
    by_step.chain(by_call).collect()
}

/// Return every publisher step that reaches CodeScene without uploading.
pub(crate) fn mode_faults(workflows: &[Workflow]) -> Vec<String> {
    contacts(workflows)
        .into_iter()
        .filter(|(workflow, _, _, operation)| {
            is_publisher(workflow) && *operation != Operation::Upload
        })
        .map(|(_, job, step, operation)| {
            format!(
                "{} step {:?} performs {operation:?}",
                job.coordinate(),
                step.label()
            )
        })
        .collect()
}

/// Return every upload step.
fn uploads(workflows: &[Workflow]) -> Vec<(&Job, &Step)> {
    contacts(workflows)
        .into_iter()
        .filter(|(.., operation)| *operation == Operation::Upload)
        .map(|(_, job, step, _)| (job, step))
        .collect()
}

/// Return where one step holds the token, when that is not exactly where an
/// upload step must.
fn step_token_fault(job: &Job, step: &Step) -> Option<String> {
    let is_upload = operation_of(step) == Some(Operation::Upload);
    let sites = step.secret_sites(TOKEN_SECRET);
    let expected = if is_upload {
        vec![TOKEN_SITE.to_owned()]
    } else {
        Vec::new()
    };
    let is_misplaced = sites != expected || (!is_upload && holds_token(step));
    is_misplaced.then(|| {
        format!(
            "{} step {:?} (upload: {is_upload}) reads the token at {sites:?}; an \
             upload step reads it at {TOKEN_SITE} alone and any other step not at all",
            job.coordinate(),
            step.label()
        )
    })
}

/// Return every scope wider than a step that reaches the token.
fn scope_token_faults(workflows: &[Workflow]) -> Vec<String> {
    let at_workflow = workflows
        .iter()
        .filter(|workflow| {
            workflow.declares_env(CODESCENE_TOKEN)
                || !workflow.secret_sites(TOKEN_SECRET).is_empty()
        })
        .map(|workflow| format!("{} reads the token at workflow level", workflow.file));
    let at_job = jobs(workflows)
        .into_iter()
        .filter(|(_, job)| {
            job.declares_env(CODESCENE_TOKEN)
                || !job.secret_sites(TOKEN_SECRET).is_empty()
                || job.inherits_secrets
        })
        .map(|(_, job)| {
            format!(
                "{} reads or forwards the token at job level",
                job.coordinate()
            )
        });
    at_workflow.chain(at_job).collect()
}

/// Return every place the token is held other than on an upload step.
///
/// Three claims, not one: the step that uploads has the token, no other step
/// has it, and no wider scope does. The first is what a rule stated only as a
/// prohibition leaves out, and it is the one that keeps the upload alive,
/// since the upload step runs only when the token is non-empty.
pub(crate) fn token_faults(workflows: &[Workflow]) -> Vec<String> {
    steps(workflows)
        .into_iter()
        .filter_map(|(_, job, step)| step_token_fault(job, step))
        .chain(scope_token_faults(workflows))
        .collect()
}

/// Return why one upload step's guard or credential is not as required.
fn guard_fault(job: &Job, step: &Step) -> Option<String> {
    let at = format!("{} step {:?}", job.coordinate(), step.label());
    let parts = match step
        .condition
        .as_deref()
        .map(|text| Condition(text).conjuncts())
    {
        None => return Some(format!("{at} has no `if` guard")),
        Some(Err(condition)) => return Some(format!("{at} guard {condition:?} has an `||`")),
        Some(Ok(parts)) => parts,
    };
    let missing: Vec<&str> = [TOKEN_GUARD, REF_GUARD]
        .into_iter()
        .filter(|required| !parts.iter().any(|part| part == required))
        .collect();
    let is_action = step.uses.is_some();
    let is_credential_wrong = is_action && step.input("access-token") != Some(TOKEN_INPUT);
    (!missing.is_empty() || is_credential_wrong).then(|| {
        format!(
            "{at} guard lacks {missing:?}, or its access-token input {:?} is not \
             {TOKEN_INPUT}",
            step.input("access-token")
        )
    })
}

/// Return every upload step whose guard or credential is not as required.
pub(crate) fn guard_faults(workflows: &[Workflow]) -> Vec<String> {
    uploads(workflows)
        .into_iter()
        .filter_map(|(job, step)| guard_fault(job, step))
        .collect()
}

/// Return every publisher scope whose concurrency cancels a running upload.
pub(crate) fn concurrency_faults(workflows: &[Workflow]) -> Vec<String> {
    let publishers: Vec<&Workflow> = workflows.iter().filter(|w| is_publisher(w)).collect();
    let at_workflow = publishers
        .iter()
        .filter(|workflow| workflow.cancels_in_progress)
        .map(|workflow| format!("{} cancels in progress", workflow.file));
    let at_job = jobs(publishers.iter().copied())
        .into_iter()
        .filter(|(_, job)| job.cancels_in_progress)
        .map(|(_, job)| format!("{} cancels in progress", job.coordinate()));
    at_workflow.chain(at_job).collect()
}

#[rstest]
fn only_the_publisher_uploads_coverage(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let workflows = repository?;
    assert!(
        !contacts(&workflows).is_empty(),
        "coverage must reach CodeScene from somewhere, or this contract \
         passes over a repository that silently stopped reporting"
    );
    let faults = placement_faults(&workflows);
    assert!(
        faults.is_empty(),
        "only the push-to-main workflow may reach CodeScene: {faults:?}"
    );
    Ok(())
}

#[rstest]
fn the_publisher_uploads_rather_than_checks(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let workflows = repository?;
    assert!(
        !uploads(&workflows).is_empty(),
        "the publisher must carry an upload step, or this contract asserts nothing"
    );
    let faults = mode_faults(&workflows);
    assert!(
        faults.is_empty(),
        "the publisher sends coverage rather than gating on it, and says so \
         explicitly; \"check\" is the pull-request mode this repository no \
         longer runs: {faults:?}"
    );
    Ok(())
}

#[rstest]
fn the_codescene_token_reaches_the_upload_step_and_nothing_else(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let faults = token_faults(&repository?);
    assert!(
        faults.is_empty(),
        "the token belongs to the upload step and to nothing else: a wider \
         scope exports it into this repository's own build, where a \
         compromised dependency reaches it: {faults:?}"
    );
    Ok(())
}

#[rstest]
fn the_upload_runs_only_on_main_with_its_token(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let faults = guard_faults(&repository?);
    assert!(
        faults.is_empty(),
        "the upload step is guarded on the token and on the main ref, as \
         separate conjuncts with no `||` to make either optional: {faults:?}"
    );
    Ok(())
}

#[rstest]
fn the_publisher_queues_rather_than_cancels(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let faults = concurrency_faults(&repository?);
    assert!(
        faults.is_empty(),
        "a cancelled publisher abandons its upload and its ratchet baseline: \
         {faults:?}"
    );
    Ok(())
}

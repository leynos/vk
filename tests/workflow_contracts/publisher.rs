//! Who owns the coverage upload, and how it is held to `main`.
//!
//! The push-to-main workflow is the only one that may reach CodeScene, it
//! uploads rather than gates, and the token is read in exactly two places:
//! the availability check's command and the upload's `access-token` input,
//! never in any `env`. The upload step is guarded twice, on the check's
//! output and on the ref, because the publisher also answers
//! `workflow_dispatch`, which runs against any branch. And the publisher's runs queue rather than cancel, so
//! a later push cannot abandon an earlier upload and its baseline.

use rstest::rstest;

use crate::repository;

use crate::codescene::{
    CHECK_SITE, CODESCENE_TOKEN, Operation, REF_GUARD, TOKEN_GUARD, TOKEN_INPUT, TOKEN_SECRET,
    TOKEN_SITE, calls_codescene, is_availability_check, is_publisher, operation_of,
};
use crate::pull_request_lanes::generators_of;
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

/// Return the one key path at which a step may read the token, if any.
fn expected_site(step: &Step) -> Option<&'static str> {
    if operation_of(step) == Some(Operation::Upload) {
        Some(TOKEN_SITE)
    } else if is_availability_check(step) {
        Some(CHECK_SITE)
    } else {
        None
    }
}

/// Return where one step holds the token, when that is not exactly where it
/// must.
///
/// An upload reads it at its `access-token` input, the availability check in
/// its command, and every other step nowhere. No step binds it in `env` under
/// any name: the uploader is a composite action whose nested steps inherit
/// the calling step's `env`, so a shell upload, which could only take the
/// token through `env` or its script, is refused as well.
fn step_token_fault(job: &Job, step: &Step) -> Option<String> {
    let expected = expected_site(step);
    let sites = step.secret_sites(TOKEN_SECRET);
    let allowed: Vec<String> = expected.map(str::to_owned).into_iter().collect();
    let is_misplaced = sites != allowed || step.env_value(CODESCENE_TOKEN).is_some();
    is_misplaced.then(|| {
        format!(
            "{} step {:?} reads the token at {sites:?}, but may read it at {expected:?} \
             alone: an upload at {TOKEN_SITE}, the availability check at {CHECK_SITE}, \
             and no step in any env",
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

/// Return every place the token is held other than where it is used.
///
/// Three claims, not one: the upload and the availability check each read
/// the token exactly where they must, no other step reads it, and no wider
/// scope does. The first is what a rule stated only as a prohibition leaves
/// out, and it is the one that keeps the upload alive, since deleting the
/// token satisfies every prohibition while the upload's guard goes false.
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
    let is_unchecked = !is_checked(job, step);
    (!missing.is_empty() || is_credential_wrong || is_unchecked).then(|| {
        format!(
            "{at} guard lacks {missing:?}, its access-token input {:?} is not \
             {TOKEN_INPUT}, or no availability check precedes it (unchecked: \
             {is_unchecked})",
            step.input("access-token")
        )
    })
}

/// Return whether the availability check runs before `upload` in its job.
///
/// The guard reads the check's output, so a check that is missing, conditional
/// or later in the job leaves the upload skipping forever.
fn is_checked(job: &Job, upload: &Step) -> bool {
    job.steps
        .iter()
        .take_while(|step| !std::ptr::eq(*step, upload))
        .any(is_availability_check)
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

/// Return why one workflow's token is wider than read-only contents.
///
/// Declared once at workflow level, and widened by no job: a job-level
/// `permissions` block replaces the workflow's rather than narrowing it.
fn permissions_fault(workflow: &Workflow) -> Option<String> {
    let declared = workflow.raw.get("permissions");
    let is_read_only = declared
        .and_then(|block| block.as_mapping())
        .is_some_and(|block| {
            block.len() == 1
                && block.get("contents").and_then(|scope| scope.as_str()) == Some("read")
        });
    let widening: Vec<&str> = workflow
        .jobs
        .iter()
        .filter(|job| job.raw.get("permissions").is_some())
        .map(|job| job.id.as_str())
        .collect();
    (!is_read_only || !widening.is_empty()).then(|| {
        format!(
            "{} declares {declared:?} at workflow level, and jobs {widening:?} \
             declare their own",
            workflow.file
        )
    })
}

/// Return every coverage workflow, and why each one's token is too wide.
pub(crate) fn permissions_faults(workflows: &[Workflow]) -> (usize, Vec<String>) {
    let coverage: Vec<&Workflow> = workflows
        .iter()
        .filter(|workflow| !generators_of(workflow).is_empty())
        .collect();
    let faults = coverage
        .iter()
        .filter_map(|workflow| permissions_fault(workflow))
        .collect();
    (coverage.len(), faults)
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
fn the_codescene_token_reaches_the_upload_input_and_nothing_else(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let faults = token_faults(&repository?);
    assert!(
        faults.is_empty(),
        "the token belongs to the upload's input and the availability check's \
         command and to nothing else: an env exports it into every nested step \
         of the composite uploader, and a wider scope into this repository's \
         own build: {faults:?}"
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
        "the upload step is guarded on the availability check's output and on \
         the main ref, as separate conjuncts with no `||` to make either \
         optional, after a check that runs unconditionally: {faults:?}"
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

#[rstest]
fn every_coverage_workflow_reads_contents_only(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let (coverage, faults) = permissions_faults(&repository?);
    assert!(
        coverage >= 2,
        "the pull-request lane and the publisher both generate coverage, or \
         this contract has lost one of its subjects"
    );
    assert!(
        faults.is_empty(),
        "a coverage workflow's token reads contents and nothing more; these \
         lanes write nothing to GitHub, and the shared actions hand the token \
         to installers: {faults:?}"
    );
    Ok(())
}

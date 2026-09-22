//! Who may contact CodeScene, who owns the coverage upload, and how
//! Markdown is linted.
//!
//! Three rules hold this repository's coverage lanes to the estate shape, and
//! none of them is visible in a green run. A pull-request lane must not reach
//! CodeScene at all, because the upload fails for an unanalysed branch and the
//! token that authenticates it has no business on the fork-facing side. The
//! push-to-main lane must be the one that uploads. And Markdown must be linted
//! through the pinned action rather than a shell invocation, whose linter
//! version is whatever the runner image happens to carry.
//!
//! Each contract derives its subject from the workflow's own triggers rather
//! than from a list of file names. A contract keyed on names passes unchanged
//! when a workflow is added, which is precisely when the question is being
//! asked again.

#[path = "support/workflow_coverage.rs"]
mod workflow_coverage;

use rstest::{fixture, rstest};
use workflow_coverage::{CODESCENE_TOKEN, Job, Step, Workflow, jobs, steps};

/// Return whether a step reaches CodeScene by any of the three routes.
fn contacts_codescene(uses: Option<&str>, run: Option<&str>) -> bool {
    let by_action = uses.is_some_and(|action| action.contains("codescene"));
    let by_command = run.is_some_and(|script| script.contains("cs-coverage"));
    by_action || by_command
}

/// Return why a job exports the token to steps that have no use for it.
fn job_token_fault(job: &Job) -> Option<String> {
    job.env.contains(CODESCENE_TOKEN).then(|| {
        format!(
            "{} exports {CODESCENE_TOKEN} into every step it runs",
            job.coordinate()
        )
    })
}

/// Return every way one step of a pull-request lane reaches CodeScene.
fn step_reach_faults(job: &Job, step: &Step) -> Vec<String> {
    let by_call = contacts_codescene(step.uses.as_deref(), step.run.as_deref()).then(|| {
        format!(
            "{} step {:?} contacts CodeScene",
            job.coordinate(),
            step.label()
        )
    });
    let by_secret = step.env.contains(CODESCENE_TOKEN).then(|| {
        format!(
            "{} step {:?} receives {CODESCENE_TOKEN}",
            job.coordinate(),
            step.label()
        )
    });
    by_call.into_iter().chain(by_secret).collect()
}

/// Return every way one pull-request lane reaches CodeScene.
///
/// The job's own environment and each of its steps, flattened into one list
/// so the assertion reports every fault at once rather than the first.
fn codescene_reach_faults(job: &Job) -> Vec<String> {
    job_token_fault(job)
        .into_iter()
        .chain(
            job.steps
                .iter()
                .flat_map(|step| step_reach_faults(job, step)),
        )
        .collect()
}

#[rstest]
fn no_pull_request_lane_contacts_codescene(workflows: Vec<Workflow>) {
    let lanes: Vec<_> = jobs(&workflows)
        .into_iter()
        .filter(|(workflow, _)| workflow.serves_pull_requests())
        .collect();
    assert!(
        !lanes.is_empty(),
        "the repository must run at least one pull-request lane, or every \
         assertion here passes over an empty set"
    );

    let at_workflow_level = workflows
        .iter()
        .filter(|workflow| {
            workflow.serves_pull_requests() && workflow.env.contains(CODESCENE_TOKEN)
        })
        .map(|workflow| {
            format!(
                "{} declares {CODESCENE_TOKEN} at workflow level, so every \
                 step of every job receives it",
                workflow.file
            )
        });
    let faults: Vec<String> = lanes
        .iter()
        .flat_map(|(_, job)| codescene_reach_faults(job))
        .chain(at_workflow_level)
        .collect();
    assert!(
        faults.is_empty(),
        "a pull-request lane must not reach CodeScene: the upload is refused \
         for a branch CodeScene does not analyse, and the token has no \
         business on the fork-facing side of the repository: {faults:?}"
    );
}

#[rstest]
fn every_pull_request_workflow_ratchets_its_own_coverage(workflows: Vec<Workflow>) {
    // A coverage lane is a workflow that generates coverage, derived rather
    // than named. Requiring every pull-request workflow to generate coverage
    // would refuse `dependabot-automerge.yml`, which answers
    // `pull_request_target` and rightly generates none.
    let lanes: Vec<&Workflow> = workflows
        .iter()
        .filter(|workflow| workflow.serves_pull_requests() && !generators_of(workflow).is_empty())
        .collect();
    assert!(
        !lanes.is_empty(),
        "a pull-request workflow must generate coverage, or this contract \
         asserts nothing and the ratchet has no subject"
    );
    // Judged per workflow rather than over one pooled list. Pooling lets a
    // second lane's ratcheting generator stand in for a lane whose own
    // generator does not ratchet, because the faults are counted together
    // and the collection is non-empty either way.
    let faults: Vec<String> = lanes
        .iter()
        .filter_map(|workflow| ratchet_fault(workflow))
        .collect();
    assert!(
        faults.is_empty(),
        "a lane generating coverage without the ratchet measures nothing it \
         can fail on, and the ratchet is the whole of what replaces the \
         CodeScene changed-line gate: {faults:?}"
    );
}

/// Return every coverage-generating step of one workflow, with its job.
fn generators_of(workflow: &Workflow) -> Vec<(&Job, &Step)> {
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

/// Return why one lane's coverage is not ratcheted, if it is not.
fn ratchet_fault(workflow: &Workflow) -> Option<String> {
    let unratcheted: Vec<String> = generators_of(workflow)
        .iter()
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
    (!unratcheted.is_empty()).then(|| unratcheted.join("; "))
}

#[rstest]
fn only_the_publisher_uploads_coverage(workflows: Vec<Workflow>) {
    let uploads: Vec<_> = steps(&workflows)
        .into_iter()
        .filter(|(_, _, step)| contacts_codescene(step.uses.as_deref(), step.run.as_deref()))
        .collect();
    assert!(
        !uploads.is_empty(),
        "coverage must reach CodeScene from somewhere, or this contract \
         passes over a repository that silently stopped reporting"
    );
    let faults: Vec<String> = uploads
        .iter()
        .filter(|(workflow, _, _)| !workflow.is_publisher())
        .map(|(workflow, job, step)| {
            format!(
                "{} step {:?} uploads, but {} answers {:?} rather than a push \
                 to the published branch alone",
                job.coordinate(),
                step.label(),
                workflow.file,
                workflow.events
            )
        })
        .collect();
    assert!(
        faults.is_empty(),
        "only the push-to-main workflow may upload coverage: {faults:?}"
    );
}

#[rstest]
fn the_publisher_uploads_rather_than_checks(workflows: Vec<Workflow>) {
    let uploads: Vec<_> = steps(&workflows)
        .into_iter()
        .filter(|(workflow, _, step)| {
            workflow.is_publisher()
                && step
                    .uses
                    .as_deref()
                    .is_some_and(|action| action.contains("codescene"))
        })
        .collect();
    assert!(
        !uploads.is_empty(),
        "the publisher must carry a CodeScene step, or this contract asserts \
         nothing"
    );
    let faults: Vec<String> = uploads
        .iter()
        .filter(|(_, _, step)| step.input("mode") != Some("upload"))
        .map(|(_, job, step)| {
            format!(
                "{} step {:?} runs in mode {:?}",
                job.coordinate(),
                step.label(),
                step.input("mode")
            )
        })
        .collect();
    assert!(
        faults.is_empty(),
        "the publisher sends coverage rather than gating on it; \"check\" is \
         the pull-request mode this repository no longer runs, and leaving \
         the mode implicit makes which one is in force unreadable: {faults:?}"
    );
}

/// Return whether a step calls the CodeScene uploader.
fn uploads_to_codescene(step: &Step) -> bool {
    step.uses
        .as_deref()
        .is_some_and(|action| action.contains("codescene"))
}

/// Return every upload step that lacks the token its own guard tests.
///
/// The positive half of the rule, and the one that was missing. The upload
/// step runs only when the token is non-empty, so a token moved elsewhere
/// leaves the guard false: the publish stops happening and a contract stating
/// only where the token may *not* be stays green.
fn upload_without_token_faults(workflows: &[Workflow]) -> Vec<String> {
    steps(workflows)
        .into_iter()
        .filter(|(_, _, step)| uploads_to_codescene(step) && !step.env.contains(CODESCENE_TOKEN))
        .map(|(_, job, step)| {
            format!(
                "{} step {:?} uploads without {CODESCENE_TOKEN}, so its guard \
                 is false and the upload never runs",
                job.coordinate(),
                step.label()
            )
        })
        .collect()
}

/// Return every step holding the token that does not upload with it.
fn token_without_upload_faults(workflows: &[Workflow]) -> Vec<String> {
    steps(workflows)
        .into_iter()
        .filter(|(_, _, step)| step.env.contains(CODESCENE_TOKEN) && !uploads_to_codescene(step))
        .map(|(_, job, step)| {
            format!(
                "{} step {:?} receives {CODESCENE_TOKEN} without uploading",
                job.coordinate(),
                step.label()
            )
        })
        .collect()
}

#[rstest]
fn the_codescene_token_reaches_the_upload_step_and_nothing_else(workflows: Vec<Workflow>) {
    let uploads = steps(&workflows)
        .into_iter()
        .filter(|(_, _, step)| uploads_to_codescene(step))
        .count();
    assert!(
        uploads > 0,
        "some step must upload to CodeScene, or this contract passes over a \
         repository that stopped reporting altogether"
    );

    // Three claims, not one. The step that needs the token has it; no other
    // step does; and no wider scope does. The first is what a rule stated
    // only as a prohibition leaves out, and it is the one that keeps the
    // upload alive.
    let faults: Vec<String> = upload_without_token_faults(&workflows)
        .into_iter()
        .chain(token_without_upload_faults(&workflows))
        .chain(wider_scope_faults(&workflows))
        .collect();
    assert!(
        faults.is_empty(),
        "the token belongs to the upload step and to nothing else: a wider \
         scope exports it into this repository's own build, where a \
         compromised dependency reaches it: {faults:?}"
    );
}

/// Return every scope wider than a step that exports the token.
fn wider_scope_faults(workflows: &[Workflow]) -> Vec<String> {
    let at_workflow_level = workflows
        .iter()
        .filter(|workflow| workflow.env.contains(CODESCENE_TOKEN))
        .map(|workflow| {
            format!(
                "{} declares {CODESCENE_TOKEN} at workflow level, reaching \
                 every step of every job",
                workflow.file
            )
        });
    let at_job_level = jobs(workflows)
        .into_iter()
        .filter(|(_, job)| job.env.contains(CODESCENE_TOKEN))
        .map(|(_, job)| {
            format!(
                "{} declares {CODESCENE_TOKEN} at job level",
                job.coordinate()
            )
        });
    at_workflow_level.chain(at_job_level).collect()
}

/// Every workflow this repository declares, parsed once per contract.
///
/// A fixture rather than a call repeated in six places: the parse is shared
/// setup, and `rstest` is how this repository expresses that.
#[fixture]
fn workflows() -> Vec<Workflow> {
    workflow_coverage::workflows()
}

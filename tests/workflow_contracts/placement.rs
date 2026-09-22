//! Where each lane runs, what it may bill, and which labels are registered.
//!
//! This repository pays per minute for its pull-request, push and tag lanes
//! and nothing for the rest. Three things therefore have to stay true
//! together, and none of them is visible in a green run: a fork's pull request
//! must still reach a runner it can have, every paid lane must carry a ceiling
//! so a hung job cannot bill for GitHub's six-hour default, and the labels in
//! use must be exactly the labels registered with actionlint.
//!
//! Each contract derives what it asserts from the workflow's own triggers
//! rather than from a list of job names. A contract keyed on names passes
//! unchanged when a job is renamed or a lane is added, which is precisely when
//! the placement question is being asked again.

use std::collections::BTreeSet;

use rstest::rstest;

use crate::reader::{Job, RunnerSelection, Workflow, WorkflowError, jobs};
use crate::registry::{HOSTED_LABELS, labels_in_use, registry};
use crate::repository;

/// The paid runner label this repository is entitled to name.
pub(crate) const UBICLOUD_LABEL: &str = "ubicloud-standard-2";

/// The measured bounds for each paid lane's `timeout-minutes`.
///
/// The lower bound holds the ceiling above the slowest run observed on a
/// four-vCPU hosted runner doubled, because these lanes move to two vCPUs and
/// a ceiling sized from the hosted figure would cancel an ordinary run. The
/// upper bound is what stops a hung lane billing for hours: the whole point of
/// declaring a ceiling is that it is lower than GitHub's six-hour default, and
/// a ceiling of several hours declares one without meaning it.
///
/// Measured from the last three green pull-request runs, the last green push
/// run, and the last three green tag runs. Queue and run seconds per job:
///
/// | job | queue | run |
/// | --- | --- | --- |
/// | `build-test` | 2 | 237, 302, 420 |
/// | `unstable-rest-resolve` | 2 | 238, 291, 440 |
/// | `coverage-upload` | 3 | 184 |
/// | `build` | 3, 2, 4 | 151, 131, 141 |
/// | `release` | 2, 2, 4 | 10, 9, 5 |
///
/// The two tag lanes are quick, and their lower bounds are well above twice
/// their slowest run rather than close to it. That is deliberate: they run a
/// handful of times a year, so there is no distribution to size against and
/// nothing is spent by leaving headroom, whereas a ceiling cancelling a
/// release build is expensive in a way no other lane here is.
pub(crate) const CEILING_BOUNDS: [(&str, u64, u64); 5] = [
    ("build-test", 15, 45),
    ("unstable-rest-resolve", 15, 45),
    ("coverage-upload", 10, 45),
    ("build", 8, 45),
    ("release", 5, 30),
];

/// Return whether a workflow runs for a fork's pull request.
///
/// `pull_request` only. `pull_request_target` runs against the base
/// repository with the base repository's secrets, so a fork cannot reach it
/// and it is never a candidate for the fork fallback.
fn is_fork_facing(workflow: &Workflow) -> bool {
    workflow.events.contains("pull_request")
}

/// Return whether a workflow is a trunk or tag lane.
///
/// Two conditions, not one. A workflow answering a push is a paid lane only
/// if it serves no pull request as well: a workflow declaring both triggers
/// owes the fork fallback, and requiring it to name the paid label outright
/// would contradict that.
fn is_trunk_or_tag(workflow: &Workflow) -> bool {
    workflow.serves_pushes() && !is_fork_facing(workflow)
}

/// Return whether a workflow answers only manual or automation events.
fn is_api_bound(workflow: &Workflow) -> bool {
    !is_fork_facing(workflow) && !workflow.serves_pushes()
}

/// Return whether every label a job can reach is GitHub-hosted.
fn is_github_hosted(job: &Job) -> bool {
    let labels = job.runs_on.labels();
    !labels.is_empty()
        && labels
            .iter()
            .all(|label| HOSTED_LABELS.contains(&label.as_str()))
}

/// Return every runner-selecting job of the workflows `select` accepts.
fn lanes(workflows: &[Workflow], select: fn(&Workflow) -> bool) -> Vec<&Job> {
    jobs(workflows)
        .into_iter()
        .filter(|(workflow, job)| select(workflow) && job.runs_on.names_a_runner())
        .map(|(_, job)| job)
        .collect()
}

/// Return why one pull-request lane's selection is wrong, if it is.
pub(crate) fn fork_fallback_fault(job: &Job) -> Option<String> {
    let RunnerSelection::Expression { raw, arms } = &job.runs_on else {
        return Some(format!(
            "{} names {} outright; a fork's pull request would find no runner",
            job.coordinate(),
            job.runs_on.raw()
        ));
    };
    if !raw.contains("github.event.pull_request.head.repo.fork") {
        return Some(format!(
            "{} chooses its runner on something other than the fork field: {raw}",
            job.coordinate()
        ));
    }
    let arms: BTreeSet<&str> = arms.iter().map(String::as_str).collect();
    let expected = BTreeSet::from(["ubuntu-latest", UBICLOUD_LABEL]);
    (arms != expected).then(|| {
        format!(
            "{} evaluates to {arms:?} rather than {expected:?}",
            job.coordinate()
        )
    })
}

/// Return why one paid lane's ceiling is wrong, if it is.
pub(crate) fn ceiling_fault(job: &Job) -> Option<String> {
    let Some(minutes) = job.timeout_minutes else {
        return Some(format!("{} declares no timeout-minutes", job.coordinate()));
    };
    let Some((_, lowest, highest)) = CEILING_BOUNDS
        .iter()
        .find(|(id, ..)| *id == job.id)
        .copied()
    else {
        return Some(format!(
            "{} is a paid lane with no measured bounds; size it from a green \
             run before placing it",
            job.coordinate()
        ));
    };
    (!(lowest..=highest).contains(&minutes)).then(|| {
        format!(
            "{} sets {minutes} minutes, outside the measured {lowest} to {highest}",
            job.coordinate()
        )
    })
}

/// Return every paid lane, and why each one's ceiling is wrong.
pub(crate) fn ceiling_faults(workflows: &[Workflow]) -> (usize, Vec<String>) {
    let paid: Vec<&Job> = jobs(workflows)
        .into_iter()
        .map(|(_, job)| job)
        .filter(|job| {
            job.runs_on
                .labels()
                .iter()
                .any(|label| label == UBICLOUD_LABEL)
        })
        .collect();
    let faults = paid.iter().filter_map(|job| ceiling_fault(job)).collect();
    (paid.len(), faults)
}

/// Return every trunk or tag lane, and each one that can reach another label.
pub(crate) fn trunk_faults(workflows: &[Workflow]) -> (usize, Vec<String>) {
    let trunk = lanes(workflows, is_trunk_or_tag);
    let faults = trunk
        .iter()
        .filter(|job| {
            job.runs_on
                .labels()
                .iter()
                .any(|label| label != UBICLOUD_LABEL)
        })
        .map(|job| {
            format!(
                "{} can reach {:?} rather than {UBICLOUD_LABEL} alone",
                job.coordinate(),
                job.runs_on.labels()
            )
        })
        .collect();
    (trunk.len(), faults)
}

#[rstest]
fn every_pull_request_lane_falls_back_for_a_fork(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let workflows = repository?;
    let fork_facing = lanes(&workflows, is_fork_facing);
    assert!(
        !fork_facing.is_empty(),
        "the repository must run at least one pull-request lane, or every \
         assertion here passes over an empty set"
    );
    let faults: Vec<String> = fork_facing
        .iter()
        .filter_map(|job| fork_fallback_fault(job))
        .collect();
    assert!(
        faults.is_empty(),
        "a fork's pull request cannot obtain a paid runner, so every \
         pull-request lane must fall back to a hosted one: {faults:?}"
    );
    Ok(())
}

#[rstest]
fn no_runner_selection_carries_an_embedded_line_break(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let workflows = repository?;
    let broken: Vec<String> = jobs(&workflows)
        .into_iter()
        .filter(|(_, job)| job.runs_on.raw().contains('\n'))
        .map(|(_, job)| format!("{}: {:?}", job.coordinate(), job.runs_on.raw()))
        .collect();
    assert!(
        broken.is_empty(),
        "a folded scalar whose continuation is indented deeper than its first \
         line keeps the break, and GitHub evaluates the broken value anyway, \
         so a green run is not evidence: {broken:?}"
    );
    Ok(())
}

#[rstest]
fn every_paid_lane_declares_a_bounded_ceiling(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let (paid, faults) = ceiling_faults(&repository?);
    assert!(
        paid > 0,
        "at least one lane must be on the paid runner, or this contract \
         asserts nothing"
    );
    assert!(
        faults.is_empty(),
        "a paid lane without a bounded ceiling inherits GitHub's six-hour \
         default and bills for it: {faults:?}"
    );
    Ok(())
}

#[rstest]
fn every_trunk_and_tag_lane_runs_on_the_paid_runner(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let (trunk, faults) = trunk_faults(&repository?);
    assert!(
        trunk > 0,
        "the repository must run at least one push or tag lane, or every \
         assertion here passes over an empty set"
    );
    assert!(
        faults.is_empty(),
        "a push or tag lane serves no fork, so it names the paid runner \
         outright rather than choosing. Without this, moving one lane back to \
         a hosted runner leaves every other contract here passing: the \
         ceiling contract inspects only lanes already on the paid runner, and \
         the registry stays balanced while the pull-request lanes keep the \
         label in use: {faults:?}"
    );
    Ok(())
}

#[rstest]
fn api_bound_lanes_stay_on_hosted_runners(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let workflows = repository?;
    let misplaced: Vec<String> = lanes(&workflows, is_api_bound)
        .into_iter()
        .filter(|job| !is_github_hosted(job))
        .map(|job| format!("{}: {}", job.coordinate(), job.runs_on.raw()))
        .collect();
    assert!(
        misplaced.is_empty(),
        "a lane answering neither a pull request nor a push is a manual or \
         automation lane: minutes are free for it on a hosted runner, so it \
         must not bill: {misplaced:?}"
    );
    Ok(())
}

#[rstest]
fn every_label_in_use_is_registered_and_every_registration_is_in_use(
    repository: Result<Vec<Workflow>, WorkflowError>,
    registry: Result<BTreeSet<String>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let in_use = labels_in_use(&repository?);
    let registered = registry?;
    assert!(
        !registered.is_empty(),
        ".github/actionlint.yaml must register the paid label, or actionlint \
         rejects the lane that names it"
    );
    assert_eq!(
        in_use, registered,
        "the registry and the labels in use must be equal in both \
         directions: an unregistered label fails actionlint, and a \
         registration left behind after its lane moved away is what lets a \
         second paid provider's label slip in without anyone deciding to pay \
         for it"
    );
    Ok(())
}

#[rstest]
fn no_required_context_interpolates_its_runner(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let workflows = repository?;
    let unstable: Vec<String> = jobs(&workflows)
        .into_iter()
        .filter(|(_, job)| job.context_source().contains("${{"))
        .map(|(_, job)| format!("{}: {}", job.coordinate(), job.context_source()))
        .collect();
    assert!(
        unstable.is_empty(),
        "a required check's context is derived from the job's name when it \
         sets one and from its identifier otherwise, so either carrying the \
         runner label changes the context when the label does, and the \
         ruleset then requires one that nothing reports: {unstable:?}"
    );
    Ok(())
}

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

#[path = "support/workflow_placement.rs"]
mod workflow_placement;

use std::collections::BTreeSet;

use workflow_placement::{
    HOSTED_LABELS, Job, RunnerSelection, UBICLOUD_LABEL, Workflow, jobs, registered_labels,
    workflows,
};

/// The measured bounds for each paid lane's `timeout-minutes`.
///
/// The lower bound holds the ceiling above the slowest run observed on a
/// four-vCPU hosted runner doubled, because these lanes move to two vCPUs and
/// a ceiling sized from the hosted figure would cancel an ordinary run. The
/// upper bound is what stops a hung lane billing for hours: the whole point of
/// declaring a ceiling is that it is lower than GitHub's six-hour default, and
/// a ceiling of several hours declares one without meaning it.
///
/// Measured 2026-09-17 from the last three green pull-request runs and the
/// last green push run, queue and run seconds per job:
///
/// | job | queue | run |
/// | --- | --- | --- |
/// | `build-test` | 2 | 237, 302, 420 |
/// | `unstable-rest-resolve` | 2 | 238, 291, 440 |
/// | `coverage-upload` | 3 | 184 |
const CEILING_BOUNDS: [(&str, u64, u64); 5] = [
    ("build-test", 15, 45),
    ("unstable-rest-resolve", 15, 45),
    ("coverage-upload", 10, 45),
    ("build", 8, 45),
    ("release", 5, 30),
];

#[test]
fn every_pull_request_lane_falls_back_for_a_fork() {
    let workflows = workflows();
    let lanes: Vec<&Job> = jobs(&workflows)
        .into_iter()
        .filter(|(workflow, job)| workflow.serves_pull_requests() && job.runs_on.names_a_runner())
        .map(|(_, job)| job)
        .collect();
    assert!(
        !lanes.is_empty(),
        "the repository must run at least one pull-request lane, or every \
         assertion here passes over an empty set"
    );
    let faults: Vec<String> = lanes
        .iter()
        .filter_map(|job| fork_fallback_fault(job))
        .collect();
    assert!(
        faults.is_empty(),
        "a fork's pull request cannot obtain a paid runner, so every \
         pull-request lane must fall back to a hosted one: {faults:?}"
    );
}

/// Return why one pull-request lane's selection is wrong, if it is.
fn fork_fallback_fault(job: &Job) -> Option<String> {
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
    let expected: BTreeSet<&str> = BTreeSet::from(["ubuntu-latest", UBICLOUD_LABEL]);
    if arms != expected {
        return Some(format!(
            "{} evaluates to {arms:?} rather than {expected:?}",
            job.coordinate()
        ));
    }
    None
}

#[test]
fn no_runner_selection_carries_an_embedded_line_break() {
    let workflows = workflows();
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
}

#[test]
fn every_paid_lane_declares_a_bounded_ceiling() {
    let workflows = workflows();
    let paid: Vec<&Job> = jobs(&workflows)
        .into_iter()
        .filter(|(_, job)| job.labels_in_use().iter().any(|l| l == UBICLOUD_LABEL))
        .map(|(_, job)| job)
        .collect();
    assert!(
        !paid.is_empty(),
        "at least one lane must be on the paid runner, or this contract \
         asserts nothing"
    );
    let faults: Vec<String> = paid.iter().filter_map(|job| ceiling_fault(job)).collect();
    assert!(
        faults.is_empty(),
        "a paid lane without a bounded ceiling inherits GitHub's six-hour \
         default and bills for it: {faults:?}"
    );
}

/// Return why one paid lane's ceiling is wrong, if it is.
fn ceiling_fault(job: &Job) -> Option<String> {
    let Some(minutes) = job.timeout_minutes else {
        return Some(format!("{} declares no timeout-minutes", job.coordinate()));
    };
    let Some((_, lowest, highest)) = CEILING_BOUNDS
        .iter()
        .find(|(id, _, _)| *id == job.id)
        .copied()
    else {
        return Some(format!(
            "{} is a paid lane with no measured bounds; size it from a green \
             run before placing it",
            job.coordinate()
        ));
    };
    if minutes < lowest || minutes > highest {
        return Some(format!(
            "{} sets {minutes} minutes, outside the measured {lowest} to \
             {highest}",
            job.coordinate()
        ));
    }
    None
}

#[test]
fn api_bound_lanes_stay_on_hosted_runners() {
    let workflows = workflows();
    let misplaced: Vec<String> = jobs(&workflows)
        .into_iter()
        .filter(|(workflow, job)| workflow.is_api_bound() && job.runs_on.names_a_runner())
        .filter(|(_, job)| !job.is_github_hosted())
        .map(|(_, job)| format!("{}: {}", job.coordinate(), job.runs_on.raw()))
        .collect();
    assert!(
        misplaced.is_empty(),
        "a lane answering neither a pull request nor a push is a manual or \
         automation lane: minutes are free for it on a hosted runner, so it \
         must not bill: {misplaced:?}"
    );
}

#[test]
fn every_label_in_use_is_registered_and_every_registration_is_in_use() {
    let workflows = workflows();
    let in_use = labels_in_use(&workflows);
    let registered = registered_labels();
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
}

/// Return every non-hosted label any job can reach.
///
/// Derived from both arms of a conditional and from every job, rather than
/// from the literal declarations alone: a lane whose fallback names a paid
/// label owes a registration exactly as a bare one does. Jobs delegating to a
/// reusable workflow are exempt, because they declare no `runs-on` at all and
/// the label is the called workflow's business.
fn labels_in_use(workflows: &[Workflow]) -> BTreeSet<String> {
    jobs(workflows)
        .into_iter()
        .filter(|(_, job)| job.runs_on.names_a_runner())
        .flat_map(|(_, job)| job.labels_in_use())
        .filter(|label| !HOSTED_LABELS.contains(&label.as_str()))
        .collect()
}

#[test]
fn no_required_context_interpolates_its_runner() {
    let workflows = workflows();
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
}

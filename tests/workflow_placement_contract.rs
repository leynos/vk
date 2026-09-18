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
fn every_trunk_and_tag_lane_runs_on_the_paid_runner() {
    let workflows = workflows();
    let lanes: Vec<&Job> = jobs(&workflows)
        .into_iter()
        .filter(|(workflow, job)| workflow.is_trunk_or_tag() && job.runs_on.names_a_runner())
        .map(|(_, job)| job)
        .collect();
    assert!(
        !lanes.is_empty(),
        "the repository must run at least one push or tag lane, or every \
         assertion here passes over an empty set"
    );
    let faults: Vec<String> = lanes
        .iter()
        .filter(|job| job.labels_in_use().iter().any(|l| l != UBICLOUD_LABEL))
        .map(|job| {
            format!(
                "{} can reach {:?} rather than {UBICLOUD_LABEL} alone",
                job.coordinate(),
                job.labels_in_use()
            )
        })
        .collect();
    assert!(
        faults.is_empty(),
        "a push or tag lane serves no fork, so it names the paid runner \
         outright rather than choosing. Without this, moving one lane back to \
         a hosted runner leaves every other contract here passing: the \
         ceiling contract inspects only lanes already on the paid runner, and \
         the registry stays balanced while the pull-request lanes keep the \
         label in use: {faults:?}"
    );
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

/// Properties of the two readings that take arbitrary input.
///
/// The contracts above are claims about five checked-in files, and a
/// generator of arbitrary workflows would not make them stronger: their
/// subject is this repository's configuration, not the space of possible
/// configurations. Two functions are different. `arms_of` parses a runner
/// expression a maintainer writes by hand, and `events_of` reads a trigger
/// block whose YAML shape varies. Both take input the repository does not
/// control, so both are stated as properties rather than as examples.
mod parser_properties {
    use proptest::prelude::*;
    use serde_norway::Value;

    use super::workflow_placement::{arms_of, events_of};

    /// A label a workflow might plausibly name, quote characters excluded.
    fn label() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9-]{0,20}"
    }

    proptest! {
        /// Every single-quoted run is an arm, whatever surrounds it.
        ///
        /// The reader splits on the quote and takes the odd positions rather
        /// than slicing, so the property that matters is that it recovers
        /// exactly the quoted runs, in order, however the expression is
        /// wrapped, indented or spaced.
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

        /// An unterminated quote yields no arm from the dangling run.
        ///
        /// A selection whose final quote is missing is malformed, and the
        /// reader must not invent an arm from the remainder. Were it to, a
        /// lane whose fallback was mistyped could still present two arms and
        /// satisfy the fork-fallback contract.
        #[test]
        fn a_dangling_quote_contributes_no_arm(
            complete in prop::collection::vec(label(), 1..4),
            dangling in label(),
        ) {
            let closed = complete
                .iter()
                .map(|arm| format!("'{arm}'"))
                .collect::<Vec<_>>()
                .join(" || ");
            let expression = format!("{closed} || '{dangling}");
            prop_assert_eq!(arms_of(&expression), complete);
        }

        /// The three YAML shapes of a trigger block read alike.
        ///
        /// A workflow may write its triggers as a mapping, a sequence or a
        /// single scalar, and one of this repository's five quotes the `on`
        /// key while the others do not. A reader that understood only one
        /// shape would report those workflows as answering nothing, and every
        /// placement contract keyed on a trigger would pass over an empty set
        /// while appearing to assert something.
        #[test]
        fn every_trigger_shape_reads_the_same(
            events in prop::collection::hash_set(label(), 1..4),
            quoted in any::<bool>(),
        ) {
            let key = if quoted { "'on'" } else { "on" };
            let names: Vec<&str> = events.iter().map(String::as_str).collect();
            let expected: std::collections::BTreeSet<String> =
                events.iter().cloned().collect();

            let as_mapping = format!(
                "{key}:\n{}\njobs: {{}}\n",
                names
                    .iter()
                    .map(|name| format!("  {name}: null"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            let as_sequence = format!("{key}: [{}]\njobs: {{}}\n", names.join(", "));

            for text in [as_mapping, as_sequence] {
                let document: Value = serde_norway::from_str(&text)
                    .expect("the generated document must parse");
                prop_assert_eq!(events_of(&document), expected.clone());
            }
        }

        /// A single scalar trigger is the one event it names.
        #[test]
        fn a_scalar_trigger_is_its_own_event(event in label(), quoted in any::<bool>()) {
            let key = if quoted { "'on'" } else { "on" };
            let text = format!("{key}: {event}\njobs: {{}}\n");
            let document: Value =
                serde_norway::from_str(&text).expect("the generated document must parse");
            prop_assert_eq!(
                events_of(&document),
                std::collections::BTreeSet::from([event])
            );
        }
    }
}

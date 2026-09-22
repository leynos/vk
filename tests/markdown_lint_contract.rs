//! How this repository lints Markdown, and with what.
//!
//! A `run:` invocation of the linter takes whatever version the runner image
//! happens to carry, which is not a pin at all. The action is pinned to a
//! commit rather than to the annotated tag object that named the same release,
//! because a tag object's SHA is immutable but is not a commit, and the
//! estate's rule asks for the commit the tag points at.
//!
//! Separate from the coverage contracts beside it: this is about Markdown
//! rather than about who may talk to CodeScene, and the two share only the
//! reader.

#[path = "support/workflow_coverage.rs"]
mod workflow_coverage;

use rstest::{fixture, rstest};
use workflow_coverage::{Workflow, steps};

/// The markdownlint action, pinned to a commit rather than a tag object.
///
/// `4580e161`, which this repository carried until this contract, is the
/// annotated tag object for v24.2.0. A tag object's SHA is immutable, so the
/// pin was not unsafe, but it is not a commit and the estate's rule asks for
/// the commit the tag points at. Held as a literal here so that a repin has
/// to be a visible edit in two places rather than one.
const MARKDOWNLINT_ACTION: &str =
    "DavidAnson/markdownlint-cli2-action@21c1be1b93ad9ed58fa840aacc3f279cde2a72ff";

#[rstest]
fn markdown_is_linted_only_through_the_pinned_action(workflows: Vec<Workflow>) {
    let all = steps(&workflows);
    let pinned: Vec<_> = all
        .iter()
        .filter(|(_, _, step)| {
            step.uses
                .as_deref()
                .is_some_and(|action| action.contains("markdownlint-cli2-action"))
        })
        .collect();
    assert!(
        !pinned.is_empty(),
        "CI must lint Markdown, or this contract passes over a repository \
         that stopped linting it"
    );

    let mispinned = pinned
        .iter()
        .filter(|(_, _, step)| step.uses.as_deref() != Some(MARKDOWNLINT_ACTION))
        .map(|(_, job, step)| {
            format!(
                "{} pins {:?} rather than the commit {MARKDOWNLINT_ACTION}",
                job.coordinate(),
                step.uses
            )
        });
    let from_a_shell = all
        .iter()
        .filter(|(_, _, step)| {
            step.run
                .as_deref()
                .is_some_and(|script| script.contains("markdownlint"))
        })
        .map(|(_, job, step)| {
            format!(
                "{} step {:?} invokes the linter from a shell, whose version \
                 is whatever the runner image carries",
                job.coordinate(),
                step.label()
            )
        });
    let faults: Vec<String> = mispinned.chain(from_a_shell).collect();
    assert!(
        faults.is_empty(),
        "Markdown is linted through the pinned action alone: {faults:?}"
    );
}

/// Every workflow this repository declares, parsed once per contract.
///
/// A fixture rather than a call repeated in six places: the parse is shared
/// setup, and `rstest` is how this repository expresses that.
#[fixture]
fn workflows() -> Vec<Workflow> {
    workflow_coverage::workflows()
}

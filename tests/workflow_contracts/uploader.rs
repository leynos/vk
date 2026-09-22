//! What the CodeScene uploader may no longer be asked for.
//!
//! At the pinned revision the shared uploader's committed `cli-manifest.json`
//! is the trust anchor for the CodeScene CLI archive, and the action *rejects*
//! a non-empty `installer-checksum` with a hard failure rather than ignoring
//! it. A workflow that still passes the repository variable therefore breaks
//! its CodeScene step the moment that variable holds anything, and the
//! variable can only ever repeat the manifest's own digest.
//!
//! Four concerns are asserted, each in its own test so a failure names the
//! defect rather than a bundle. Every one of them ranges over a collection
//! whose contents are checked first: a contract over an empty collection is
//! satisfied by deleting the thing it guards.

use rstest::rstest;

use crate::reader::{Workflow, WorkflowError, jobs, steps};
use crate::repository;

/// The one approved revision of the shared uploader.
///
/// Asserted as an allowlist rather than as a floor. Ordering two commit SHAs
/// cannot be computed from a checkout, so naming the approved revision is what
/// keeps the contract hermetic; it fails closed on any other value, including
/// a tag or a branch name.
const APPROVED_PIN: &str = "a5765019912a8ab6882b12db049c7cde635f3a85";

/// The uploader reference, without its revision. The `@` separator is part of
/// the marker so a differently owned action whose path merely starts with the
/// same text cannot match.
const UPLOADER_MARKER: &str = "leynos/shared-actions/.github/actions/upload-codescene-coverage@";

/// The deprecated input. It carried the SHA-256 of an installer script the
/// action no longer downloads.
const DEPRECATED_INPUT: &str = "installer-checksum";

/// The repository variable whose only consumer was that input.
const DEPRECATED_VARIABLE: &str = "CODESCENE_CLI_SHA256";

/// The `workflow_dispatch` that hashed the installer script and wrote the
/// variable back through the API, named without an extension. Other
/// repositories in the estate carry it; this contract keeps it from arriving
/// here under any extension GitHub reads.
const REFRESH_WORKFLOW_STEM: &str = "get-codescene-sha";

/// Return the revision of every uploader reference, with the job naming it.
///
/// Read from every step's `uses` and every job's reusable-workflow call, so a
/// reference is found wherever GitHub would resolve one.
fn uploader_references(workflows: &[Workflow]) -> Vec<(String, String)> {
    let from_steps = steps(workflows)
        .into_iter()
        .filter_map(|(_, job, step)| Some((job.coordinate(), step.uses.as_deref()?)));
    let from_calls = jobs(workflows)
        .into_iter()
        .filter_map(|(_, job)| Some((job.coordinate(), job.calls.as_deref()?)));
    from_steps
        .chain(from_calls)
        .filter_map(|(coordinate, uses)| {
            Some((coordinate, uses.strip_prefix(UPLOADER_MARKER)?.to_owned()))
        })
        .collect()
}

/// Return the workflows whose text names `needle`, comments included.
///
/// Shared by the two absence contracts below. They assert different things
/// and fail apart, but the search itself is one operation.
fn workflows_naming<'a>(workflows: &'a [Workflow], needle: &str) -> Vec<&'a str> {
    workflows
        .iter()
        .filter(|workflow| workflow.text.contains(needle))
        .map(|workflow| workflow.file.as_str())
        .collect()
}

/// Return whether a workflow file is the refresh dispatch, under any
/// extension and in any letter case.
///
/// The loader reads `.YML` as readily as `.yml`, so the stem is compared the
/// same way: compared case-sensitively, `Get-CodeScene-SHA.yml` would load as
/// a workflow and pass this contract.
fn is_refresh_workflow(file: &str) -> bool {
    file.rsplit_once('.')
        .is_some_and(|(stem, _)| stem.eq_ignore_ascii_case(REFRESH_WORKFLOW_STEM))
}

/// The uploader rejects a non-empty value, so no workflow may pass it.
///
/// This is not tidying. The action fails its own input validation on a
/// non-empty value, so the CodeScene step stops working the moment the
/// variable behind the input holds anything.
#[rstest]
fn no_workflow_passes_the_deprecated_installer_checksum(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let workflows = repository?;
    let offenders = workflows_naming(&workflows, DEPRECATED_INPUT);
    assert!(
        offenders.is_empty(),
        "{DEPRECATED_INPUT} is deprecated and rejected by the uploader at \
         {APPROVED_PIN}; remove it from {offenders:?}"
    );
    Ok(())
}

/// The variable existed only to feed the rejected input, so it must go.
///
/// Held apart from the input contract because the two regress apart: an `env`
/// line or a guard can name the variable in a workflow that passes no input at
/// all, and such a reference is what a later reader would take as evidence
/// that the variable is still wanted.
#[rstest]
fn no_workflow_references_the_deprecated_checksum_variable(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let workflows = repository?;
    let offenders = workflows_naming(&workflows, DEPRECATED_VARIABLE);
    assert!(
        offenders.is_empty(),
        "{DEPRECATED_VARIABLE} fed {DEPRECATED_INPUT} and has no remaining \
         consumer; remove it from {offenders:?}"
    );
    Ok(())
}

/// One approved revision, so a stale pin cannot reintroduce the input.
///
/// The references are checked for content before they are checked for
/// compliance. Deleting the CodeScene step would otherwise satisfy this
/// contract instead of failing it.
#[rstest]
fn every_uploader_reference_is_pinned_to_the_approved_revision(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let references = uploader_references(&repository?);
    assert!(
        !references.is_empty(),
        "no upload-codescene-coverage reference was found, so the pin \
         assertion would pass vacuously; this repository is expected to call \
         the uploader"
    );
    let wrong: Vec<&(String, String)> = references
        .iter()
        .filter(|(_, revision)| revision != APPROVED_PIN)
        .collect();
    assert!(
        wrong.is_empty(),
        "every upload-codescene-coverage reference must be pinned to \
         {APPROVED_PIN}; found {wrong:?}"
    );
    Ok(())
}

/// Nothing reads the variable it would write, so it is dead code here.
///
/// Asserted over the workflow files the loader found rather than over their
/// jobs: a dispatch-only workflow appears in no job or step another contract
/// reads, so its absence is the only property that can be stated. The loader
/// reads every extension GitHub does, in any letter case, so a placeholder
/// under the other extension cannot slip past.
#[rstest]
fn the_checksum_refresh_workflow_is_absent(
    repository: Result<Vec<Workflow>, WorkflowError>,
) -> Result<(), WorkflowError> {
    let workflows = repository?;
    let present: Vec<&str> = workflows
        .iter()
        .map(|workflow| workflow.file.as_str())
        .filter(|file| is_refresh_workflow(file))
        .collect();
    assert!(
        present.is_empty(),
        "{present:?} maintains {DEPRECATED_VARIABLE}, which no workflow \
         reads; delete it rather than keeping a dispatch that writes an \
         unread repository variable"
    );
    Ok(())
}

#[rstest]
#[case::yml("get-codescene-sha.yml", true)]
#[case::yaml("get-codescene-sha.yaml", true)]
#[case::upper_case("Get-CodeScene-SHA.YML", true)]
#[case::other_stem("get-codescene-sha-v2.yml", false)]
#[case::coverage("coverage.yml", false)]
fn the_refresh_workflow_is_recognized_by_stem(#[case] file: &str, #[case] expected: bool) {
    assert_eq!(is_refresh_workflow(file), expected);
}

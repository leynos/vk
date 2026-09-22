//! The runner labels registered with actionlint, and the labels in use.
//!
//! actionlint rejects any `runs-on` label it does not know, so a paid runner
//! has to be registered in `.github/actionlint.yaml` before a lane can name
//! it. The registry is held equal to the labels in use in both directions.

use std::collections::BTreeSet;

use cap_std::ambient_authority;
use cap_std::fs_utf8::Dir;
use rstest::fixture;
use serde_norway::Value;

use crate::reader::{Workflow, WorkflowError, jobs};

/// The file, relative to `.github`, that registers runner labels.
const REGISTRY: &str = "actionlint.yaml";

/// The GitHub-hosted labels a lane may name, frozen rather than derived.
///
/// Deriving "hosted" from a prefix would let a second paid provider's labels
/// pass as hosted and so escape the registry question entirely, which is the
/// substantive defect the registry contract exists to catch. Naming them makes
/// adding a provider a visible edit here.
pub(crate) const HOSTED_LABELS: [&str; 4] = [
    "ubuntu-latest",
    "ubuntu-24.04",
    "windows-latest",
    "macos-latest",
];

/// Return the labels `directory`'s actionlint configuration registers.
///
/// ```ignore
/// let labels = registered_labels(&github_directory)?;
/// assert!(labels.contains("ubicloud-standard-2"));
/// ```
///
/// # Errors
///
/// Returns [`WorkflowError::Read`] or [`WorkflowError::Parse`] when the
/// configuration cannot be read or is not YAML.
pub(crate) fn registered_labels(directory: &Dir) -> Result<BTreeSet<String>, WorkflowError> {
    let text = directory
        .read_to_string(REGISTRY)
        .map_err(|source| WorkflowError::Read {
            file: REGISTRY.to_owned(),
            source,
        })?;
    let document: Value = serde_norway::from_str(&text).map_err(|source| WorkflowError::Parse {
        file: REGISTRY.to_owned(),
        source,
    })?;
    Ok(
        match document
            .get("self-hosted-runner")
            .and_then(|runner| runner.get("labels"))
        {
            Some(Value::Sequence(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => BTreeSet::new(),
        },
    )
}

/// Return every non-hosted label any job can reach.
///
/// Derived from both arms of a conditional and from every job, rather than
/// from the literal declarations alone: a lane whose fallback names a paid
/// label owes a registration exactly as a bare one does. Jobs delegating to a
/// reusable workflow are exempt, because they declare no `runs-on` at all and
/// the label is the called workflow's business.
pub(crate) fn labels_in_use(workflows: &[Workflow]) -> BTreeSet<String> {
    jobs(workflows)
        .into_iter()
        .filter(|(_, job)| job.runs_on.names_a_runner())
        .flat_map(|(_, job)| job.runs_on.labels())
        .filter(|label| !HOSTED_LABELS.contains(&label.as_str()))
        .collect()
}

/// The labels this repository registers with actionlint.
///
/// Ambient authority is taken here, at the boundary, for the `.github`
/// directory alone.
#[fixture]
pub(crate) fn registry() -> Result<BTreeSet<String>, WorkflowError> {
    let directory = Dir::open_ambient_dir(
        concat!(env!("CARGO_MANIFEST_DIR"), "/.github"),
        ambient_authority(),
    )
    .map_err(|source| WorkflowError::List { source })?;
    registered_labels(&directory)
}

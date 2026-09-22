//! Which workflows run on behalf of a pull request.
//!
//! A workflow runs for a pull request when its own triggers say so, and also
//! when a job of such a workflow calls it as a reusable workflow. The second
//! kind declares only `workflow_call`, so a contract enumerating workflows by
//! trigger alone never looks inside it, while `secrets: inherit` hands it
//! every secret the caller holds. Every pull-request rule therefore runs over
//! the closure computed here, not over the triggered set.

use super::load::WorkflowError;
use super::model::{Job, Workflow};

/// The directory, relative to the repository root, that holds its workflows.
const WORKFLOW_DIRECTORY: &str = ".github/workflows/";

/// Return the workflow file a job-level `uses:` names in this repository.
///
/// A local call is recognized by shape rather than by a list of prefixes:
/// strip a leading `./`, then ask whether the rest is a path under the
/// workflow directory. A call into another repository starts with its owner
/// and so never matches.
///
/// ```ignore
/// assert_eq!(local_call_target("./.github/workflows/probe.yml"), Some("probe.yml"));
/// assert_eq!(local_call_target("leynos/shared-actions/.github/workflows/x.yml@abc"), None);
/// ```
pub(crate) fn local_call_target(uses: &str) -> Option<&str> {
    uses.strip_prefix("./")
        .unwrap_or(uses)
        .strip_prefix(WORKFLOW_DIRECTORY)
}

/// Return the workflow `job` calls locally, or why it names none.
fn callee<'a>(
    workflows: &'a [Workflow],
    job: &Job,
    target: &str,
) -> Result<&'a Workflow, WorkflowError> {
    workflows
        .iter()
        .find(|workflow| workflow.file == target)
        .ok_or_else(|| WorkflowError::UnresolvedCall {
            caller: job.coordinate(),
            target: target.to_owned(),
        })
}

/// Return every workflow `workflow` calls within this repository.
fn local_callees<'a>(
    workflows: &'a [Workflow],
    workflow: &Workflow,
) -> Result<Vec<&'a Workflow>, WorkflowError> {
    workflow
        .jobs
        .iter()
        .filter_map(|job| {
            let target = job.calls.as_deref().and_then(local_call_target)?;
            Some(callee(workflows, job, target))
        })
        .collect()
}

/// Return every workflow that runs on behalf of a pull request.
///
/// The triggered workflows, then everything they call locally, transitively.
/// A call to a workflow file that does not exist is an error rather than a
/// dead end, since the contract cannot vouch for a workflow it cannot read.
///
/// ```ignore
/// let lanes = pull_request_closure(&workflows)?;
/// assert!(lanes.iter().any(|workflow| workflow.file == "probe.yml"));
/// ```
///
/// # Errors
///
/// Returns [`WorkflowError::UnresolvedCall`] for a local call naming a file
/// that is not among `workflows`.
pub(crate) fn pull_request_closure(
    workflows: &[Workflow],
) -> Result<Vec<&Workflow>, WorkflowError> {
    let mut reached: Vec<&Workflow> = workflows
        .iter()
        .filter(|workflow| workflow.serves_pull_requests())
        .collect();
    let mut frontier = reached.clone();
    while let Some(workflow) = frontier.pop() {
        for found in local_callees(workflows, workflow)? {
            if !reached.iter().any(|seen| seen.file == found.file) {
                reached.push(found);
                frontier.push(found);
            }
        }
    }
    Ok(reached)
}

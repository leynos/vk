//! Which workflows run on behalf of a pull request.
//!
//! A workflow runs for a pull request when its own triggers say so, and also
//! when a job of such a workflow calls it as a reusable workflow. The second
//! kind declares only `workflow_call`, so a contract enumerating workflows by
//! trigger alone never looks inside it, while `secrets: inherit` hands it
//! every secret the caller holds. Every pull-request rule therefore runs over
//! the closure computed here, not over the triggered set.

use super::expression::local_call_target;
use super::load::WorkflowError;
use super::model::Workflow;

/// Return the workflow a job's local call names, or why it names none.
fn callee<'a>(
    workflows: &'a [Workflow],
    caller: String,
    target: &str,
) -> Result<&'a Workflow, WorkflowError> {
    workflows
        .iter()
        .find(|workflow| workflow.file == target)
        .ok_or_else(|| WorkflowError::UnresolvedCall {
            caller,
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
            Some(callee(workflows, job.coordinate(), target))
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

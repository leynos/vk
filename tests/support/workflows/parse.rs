//! Turning one workflow's text into a [`Workflow`].
//!
//! Parsing is kept apart from reading files, so every shape the reader must
//! handle can be tested from a string, and the filesystem is only ever asked
//! for text.

use std::collections::BTreeSet;

use serde_norway::Value;

use super::load::WorkflowError;
use super::model::{Job, Pair, Step, Workflow};

/// Return one workflow, parsed from its text.
///
/// The parser refuses a document that declares a key twice in one mapping,
/// where a lenient reader keeps the last value. A workflow declaring
/// `runs-on` twice would otherwise be read with one of its values discarded
/// in silence.
///
/// ```ignore
/// let workflow = parse_workflow("ci.yml", "on: [push]\njobs: {}\n")?;
/// assert!(workflow.serves_pushes());
/// ```
///
/// # Errors
///
/// Returns [`WorkflowError::Parse`] for text that is not YAML, or that
/// repeats a key, and [`WorkflowError::Shape`] for YAML that is not a
/// mapping.
pub(crate) fn parse_workflow(file: &str, text: &str) -> Result<Workflow, WorkflowError> {
    let document: Value = serde_norway::from_str(text).map_err(|source| WorkflowError::Parse {
        file: file.to_owned(),
        source,
    })?;
    if !document.is_mapping() {
        return Err(WorkflowError::Shape {
            file: file.to_owned(),
        });
    }
    let push = triggers_of(&document).and_then(|on| on.get("push"));
    Ok(Workflow {
        file: file.to_owned(),
        events: names_of(triggers_of(&document)),
        env: pairs_of(document.get("env")),
        push_branches: names_of(push.and_then(|filter| filter.get("branches"))),
        push_tags: names_of(push.and_then(|filter| filter.get("tags"))),
        cancels_in_progress: cancels_in_progress(document.get("concurrency")),
        jobs: jobs_of(file, &document),
        raw: document,
    })
}

/// Return the names a node spells, whichever of the three shapes it takes.
///
/// GitHub writes a set of names as a mapping, as a sequence or as a bare
/// scalar depending on the key and the author. One reader rather than one per
/// key, so a shape handled in one place cannot be forgotten in another: a
/// reader that understood only mappings would read `on: [push, pull_request]`
/// as one event with a strange name, and a branch filter written
/// `branches: main` as no filter at all.
fn names_of(node: Option<&Value>) -> BTreeSet<String> {
    match node {
        Some(Value::Mapping(map)) => map
            .keys()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::Sequence(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(one)) => BTreeSet::from([one.clone()]),
        _ => BTreeSet::new(),
    }
}

/// Return a workflow's trigger block, under either spelling of the key.
///
/// `on` is looked up as a string and as the boolean it becomes under a YAML
/// 1.1 reader. This parser reads YAML 1.2, where `on` stays a string, but a
/// file written through a 1.1 tool can arrive with the key already turned into
/// `true`, and a reader finding neither would report the workflow as answering
/// nothing.
fn triggers_of(document: &Value) -> Option<&Value> {
    document
        .get("on")
        .or_else(|| document.get(Value::Bool(true)))
}

/// Return a mapping's entries as text.
///
/// Every scalar is rendered rather than only the strings, because a workflow
/// may write `mode: upload` unquoted and `retries: 3` as a number, and a
/// reader taking only strings would report such an entry as absent.
fn pairs_of(node: Option<&Value>) -> Vec<Pair> {
    let Some(Value::Mapping(map)) = node else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(key, value)| {
            let rendered = match value {
                Value::String(text) => text.clone(),
                Value::Bool(flag) => flag.to_string(),
                Value::Number(number) => number.to_string(),
                _ => return None,
            };
            Some((key.as_str()?.to_owned(), rendered))
        })
        .collect()
}

/// Return whether a concurrency block cancels the run it supersedes.
///
/// Anything other than an absent or literally false `cancel-in-progress`
/// counts as cancelling, an expression included: whether it cancels is then
/// decided at run time, and a contract cannot promise it never will.
fn cancels_in_progress(concurrency: Option<&Value>) -> bool {
    match concurrency.and_then(|block| block.get("cancel-in-progress")) {
        None | Some(Value::Bool(false)) => false,
        Some(Value::String(text)) => text != "false",
        Some(_) => true,
    }
}

/// Return the text at `key` in `node`, when it is text.
fn text_at(node: &Value, key: &str) -> Option<String> {
    node.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Return the steps of one job.
fn steps_of(job: &Value) -> Vec<Step> {
    let Some(Value::Sequence(items)) = job.get("steps") else {
        return Vec::new();
    };
    items
        .iter()
        .map(|step| Step {
            name: text_at(step, "name"),
            uses: text_at(step, "uses"),
            run: text_at(step, "run"),
            condition: text_at(step, "if"),
            with: pairs_of(step.get("with")),
            env: pairs_of(step.get("env")),
            raw: step.clone(),
        })
        .collect()
}

/// Return one job, read from its node.
fn job_of(file: &str, id: &str, job: &Value) -> Job {
    Job {
        workflow: file.to_owned(),
        id: id.to_owned(),
        env: pairs_of(job.get("env")),
        calls: text_at(job, "uses"),
        inherits_secrets: job.get("secrets").and_then(Value::as_str) == Some("inherit"),
        cancels_in_progress: cancels_in_progress(job.get("concurrency")),
        steps: steps_of(job),
        raw: job.clone(),
    }
}

/// Return every job a workflow declares.
fn jobs_of(file: &str, document: &Value) -> Vec<Job> {
    let Some(Value::Mapping(map)) = document.get("jobs") else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(id, job)| id.as_str().map(|id| job_of(file, id, job)))
        .collect()
}

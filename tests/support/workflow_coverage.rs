//! Reading the workflow files the coverage-shape contracts assert against.
//!
//! The contracts need three things the raw YAML does not hand over: which
//! events a workflow answers, every step of every job with the inputs and
//! environment it declares, and the environment a job exports into all of its
//! steps. Each is derived here so the assertions beside them read as claims
//! rather than as parsing.

// This module is included by `#[path]` into two test binaries, and each uses
// a subset of it: the coverage contracts read events, environments and step
// inputs, while the Markdown contract reads only steps. Every item below is
// live in one binary or the other, so the compiler's per-binary view of "dead"
// is not a finding here. `expect` cannot be used, because the expectation
// would go unfulfilled in whichever binary does use the item.
#![allow(
    dead_code,
    reason = "shared by two test binaries; every item is live in one of them"
)]

use std::collections::BTreeSet;

use camino::{Utf8Path, Utf8PathBuf};
use cap_std::ambient_authority;
use cap_std::fs_utf8::Dir;
use serde_norway::Value;

/// The directory holding this repository's own workflows.
pub(crate) const WORKFLOWS: &str = ".github/workflows";

/// The secret that authenticates a CodeScene upload.
pub(crate) const CODESCENE_TOKEN: &str = "CS_ACCESS_TOKEN";

/// The branch whose coverage CodeScene analyses, and so the only one that may
/// be published from.
pub(crate) const PUBLISHED_BRANCH: &str = "main";

/// One step of one job, as the contracts need to see it.
#[derive(Debug, Clone)]
pub(crate) struct Step {
    /// The step's name, when it declares one.
    pub(crate) name: Option<String>,
    /// The action the step calls, when it calls one.
    pub(crate) uses: Option<String>,
    /// The shell the step runs, when it runs one.
    pub(crate) run: Option<String>,
    /// The inputs the step passes, flattened to strings.
    pub(crate) with: Vec<(String, String)>,
    /// The environment names the step declares.
    pub(crate) env: BTreeSet<String>,
}

impl Step {
    /// Return one input's value, when the step passes it.
    pub(crate) fn input(&self, key: &str) -> Option<&str> {
        self.with
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    /// Return how the step is identified in a message.
    pub(crate) fn label(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.uses.clone())
            .unwrap_or_else(|| "an unnamed step".to_owned())
    }
}

/// One job, as the contracts need to see it.
#[derive(Debug, Clone)]
pub(crate) struct Job {
    /// The file the job is declared in.
    pub(crate) workflow: String,
    /// The job's identifier.
    pub(crate) id: String,
    /// The environment names the job exports into every one of its steps.
    pub(crate) env: BTreeSet<String>,
    /// Every step the job declares.
    pub(crate) steps: Vec<Step>,
}

impl Job {
    /// Return the job's coordinate, for a message that names the fault.
    pub(crate) fn coordinate(&self) -> String {
        format!("{}:{}", self.workflow, self.id)
    }
}

/// One workflow file and the events it answers.
#[derive(Debug, Clone)]
pub(crate) struct Workflow {
    /// The file name, for a message that names the fault.
    pub(crate) file: String,
    /// The events the workflow answers.
    pub(crate) events: BTreeSet<String>,
    /// The environment names declared at workflow level.
    ///
    /// GitHub exports these into every step of every job, so a secret here
    /// has the widest reach a workflow can give it, and a reader that saw
    /// only job and step scope would call such a workflow clean.
    pub(crate) env: BTreeSet<String>,
    /// The branches a `push` trigger is filtered to, when it names any.
    pub(crate) push_branches: BTreeSet<String>,
    /// Every job the workflow declares.
    pub(crate) jobs: Vec<Job>,
}

impl Workflow {
    /// Return whether this workflow runs for a pull request.
    ///
    /// `pull_request_target` counts. It runs against the base repository with
    /// the base repository's secrets, which is the more dangerous of the two
    /// places to put a token, not the safer one.
    pub(crate) fn serves_pull_requests(&self) -> bool {
        self.events.contains("pull_request") || self.events.contains("pull_request_target")
    }

    /// Return whether this workflow answers a push to a branch.
    pub(crate) fn serves_pushes(&self) -> bool {
        self.events.contains("push")
    }

    /// Return whether this workflow is the coverage publisher.
    ///
    /// Three conditions, not one. It answers a push; it serves no pull
    /// request, since a workflow declaring both triggers would otherwise be
    /// required to upload and forbidden from uploading at once; and its push
    /// trigger is filtered to `main`.
    ///
    /// The branch filter is what makes the rule mean anything. A tag-triggered
    /// workflow answers `push` and serves no pull request, so without it
    /// `release.yml` qualified as the publisher and could have carried a
    /// CodeScene upload with no contract objecting. CodeScene accepts an
    /// upload only for a branch it analyses, so a tag upload would fail at
    /// run time rather than be caught here.
    pub(crate) fn is_publisher(&self) -> bool {
        self.serves_pushes()
            && !self.serves_pull_requests()
            && self.push_branches.contains(PUBLISHED_BRANCH)
    }
}

/// Return the names a mapping declares, or nothing when it is not one.
fn keys_of(node: Option<&Value>) -> BTreeSet<String> {
    match node {
        Some(Value::Mapping(map)) => map
            .keys()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => BTreeSet::new(),
    }
}

/// Return a step's inputs as strings.
///
/// Every scalar is rendered rather than only the strings, because a
/// workflow may write `mode: upload` unquoted and a reader taking only
/// `as_str` would report the input as absent rather than as its value.
fn inputs_of(node: Option<&Value>) -> Vec<(String, String)> {
    let Some(Value::Mapping(map)) = node else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(key, value)| {
            let key = key.as_str()?.to_owned();
            let rendered = match value {
                Value::String(text) => text.clone(),
                Value::Bool(flag) => flag.to_string(),
                Value::Number(number) => number.to_string(),
                _ => return None,
            };
            Some((key, rendered))
        })
        .collect()
}

/// Return the steps of one job.
fn steps_of(job: &Value) -> Vec<Step> {
    let Some(Value::Sequence(items)) = job.get("steps") else {
        return Vec::new();
    };
    items
        .iter()
        .map(|step| Step {
            name: step.get("name").and_then(Value::as_str).map(str::to_owned),
            uses: step.get("uses").and_then(Value::as_str).map(str::to_owned),
            run: step.get("run").and_then(Value::as_str).map(str::to_owned),
            with: inputs_of(step.get("with")),
            env: keys_of(step.get("env")),
        })
        .collect()
}

/// Return the events a workflow answers.
///
/// `on` is looked up as a string and as the boolean it becomes under a YAML
/// 1.1 reader, because `on` is a boolean key in that schema and this
/// repository quotes the key in one workflow and not in the others. A reader
/// finding neither would report every workflow as answering nothing, and
/// every assertion below would then pass over an empty set.
fn events_of(document: &Value) -> BTreeSet<String> {
    let triggers = document
        .get("on")
        .or_else(|| document.get(Value::Bool(true)));
    match triggers {
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

/// Return the branches a workflow's `push` trigger is filtered to.
///
/// An unfiltered `push` returns nothing rather than every branch: a trigger
/// naming no filter fires for all of them, which is not the same as being
/// restricted to the published one, and the publisher rule wants the
/// restriction stated rather than inferred.
fn push_branches_of(document: &Value) -> BTreeSet<String> {
    let triggers = document
        .get("on")
        .or_else(|| document.get(Value::Bool(true)));
    match triggers
        .and_then(|on| on.get("push"))
        .and_then(|push| push.get("branches"))
    {
        Some(Value::Sequence(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(one)) => BTreeSet::from([one.clone()]),
        _ => BTreeSet::new(),
    }
}

/// Return a capability for the directory holding this repository's workflows.
///
/// Ambient authority is taken once, here, and everything below reads through
/// the returned capability rather than through a path. That is this
/// repository's rule for filesystem access generally, and it means a contract
/// cannot accidentally read outside the directory it reasons about.
fn workflow_directory() -> Dir {
    let root = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let directory = root.join(WORKFLOWS);
    Dir::open_ambient_dir(directory.as_str(), ambient_authority())
        .unwrap_or_else(|err| panic!("{directory} must be an openable directory: {err}"))
}

/// Return the name of every workflow file this repository owns, sorted.
fn workflow_names(directory: &Dir) -> Vec<String> {
    let entries = directory
        .entries()
        .unwrap_or_else(|err| panic!("{WORKFLOWS} must be readable: {err}"));
    let mut names: Vec<String> = entries
        // An entry that cannot be read is a fault, not an absence. Discarding
        // it would shrink the set every contract below iterates over, and a
        // contract that silently inspects four workflows where there are five
        // reports success for the one it never saw.
        .map(|entry| entry.unwrap_or_else(|err| panic!("{WORKFLOWS} must list cleanly: {err}")))
        .map(|entry| {
            entry
                .file_name()
                .unwrap_or_else(|err| panic!("{WORKFLOWS} entries must have UTF-8 names: {err}"))
        })
        // Compared through camino's extension reader rather than by suffix:
        // a suffix test is case-sensitive, and GitHub reads `.YML` as readily
        // as `.yml`, so a workflow named that way would be skipped in silence.
        .filter(|name| {
            Utf8Path::new(name).extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("yml") || ext.eq_ignore_ascii_case("yaml")
            })
        })
        .collect();
    names.sort();
    assert!(!names.is_empty(), "the repository must declare workflows");
    names
}

/// Return one workflow, parsed, read through the directory capability.
fn workflow_at(directory: &Dir, file: &str) -> Workflow {
    let text = directory
        .read_to_string(file)
        .unwrap_or_else(|err| panic!("{file} must be readable: {err}"));
    let document: Value = serde_norway::from_str(&text)
        .unwrap_or_else(|err| panic!("{file} must be valid YAML: {err}"));
    let jobs = match document.get("jobs") {
        Some(Value::Mapping(map)) => map
            .iter()
            .filter_map(|(id, job)| id.as_str().map(|id| (id, job)))
            .map(|(id, job)| Job {
                workflow: file.to_owned(),
                id: id.to_owned(),
                env: keys_of(job.get("env")),
                steps: steps_of(job),
            })
            .collect(),
        _ => Vec::new(),
    };
    Workflow {
        file: file.to_owned(),
        events: events_of(&document),
        env: keys_of(document.get("env")),
        push_branches: push_branches_of(&document),
        jobs,
    }
}

/// Return every workflow this repository owns, parsed.
pub(crate) fn workflows() -> Vec<Workflow> {
    let directory = workflow_directory();
    workflow_names(&directory)
        .iter()
        .map(|name| workflow_at(&directory, name))
        .collect()
}

/// Return every job of every workflow, with its workflow beside it.
pub(crate) fn jobs(workflows: &[Workflow]) -> Vec<(&Workflow, &Job)> {
    workflows
        .iter()
        .flat_map(|workflow| workflow.jobs.iter().map(move |job| (workflow, job)))
        .collect()
}

/// Return every step of every job, with its workflow and job beside it.
pub(crate) fn steps(workflows: &[Workflow]) -> Vec<(&Workflow, &Job, &Step)> {
    jobs(workflows)
        .into_iter()
        .flat_map(|(workflow, job)| job.steps.iter().map(move |step| (workflow, job, step)))
        .collect()
}

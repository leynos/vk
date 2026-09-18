//! Reading the workflow files the coverage-shape contracts assert against.
//!
//! The contracts need three things the raw YAML does not hand over: which
//! events a workflow answers, every step of every job with the inputs and
//! environment it declares, and the environment a job exports into all of its
//! steps. Each is derived here so the assertions beside them read as claims
//! rather than as parsing.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_norway::Value;

/// The directory holding this repository's own workflows.
pub(crate) const WORKFLOWS: &str = ".github/workflows";

/// The secret that authenticates a CodeScene upload.
pub(crate) const CODESCENE_TOKEN: &str = "CS_ACCESS_TOKEN";

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
    /// Two conditions, not one. A workflow that answers a push **and** serves
    /// no pull request is the publisher; a repository whose one workflow
    /// declares both triggers would otherwise be required to upload and
    /// forbidden from uploading at the same time.
    pub(crate) fn is_publisher(&self) -> bool {
        self.serves_pushes() && !self.serves_pull_requests()
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

/// Return the repository root, from this crate's manifest directory.
fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Return every workflow file this repository owns.
fn workflow_paths() -> Vec<PathBuf> {
    let directory = repository_root().join(WORKFLOWS);
    let mut paths: Vec<PathBuf> = fs::read_dir(&directory)
        .unwrap_or_else(|err| panic!("{} must be readable: {err}", directory.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "yml" || e == "yaml"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "the repository must declare workflows");
    paths
}

/// Return one workflow, parsed.
fn workflow_at(path: &Path) -> Workflow {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("{} must be readable: {err}", path.display()));
    let document: Value = serde_norway::from_str(&text)
        .unwrap_or_else(|err| panic!("{} must be valid YAML: {err}", path.display()));
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    let jobs = match document.get("jobs") {
        Some(Value::Mapping(map)) => map
            .iter()
            .filter_map(|(id, job)| id.as_str().map(|id| (id, job)))
            .map(|(id, job)| Job {
                workflow: file.clone(),
                id: id.to_owned(),
                env: keys_of(job.get("env")),
                steps: steps_of(job),
            })
            .collect(),
        _ => Vec::new(),
    };
    Workflow {
        file,
        events: events_of(&document),
        jobs,
    }
}

/// Return every workflow this repository owns, parsed.
pub(crate) fn workflows() -> Vec<Workflow> {
    workflow_paths()
        .iter()
        .map(|path| workflow_at(path))
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

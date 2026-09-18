//! Reading the workflow files the placement contracts assert against.
//!
//! The contracts need three things the raw YAML does not hand over: which
//! events a workflow answers, which runner labels a job can actually end up
//! on, and whether a job selects a runner at all. Each is derived here so the
//! assertions beside them read as claims rather than as parsing.
//!
//! Every reading is taken from the parsed document rather than from the file's
//! lines. A `runs-on` written as a folded scalar whose continuation is
//! indented one level too deep keeps its line break, and GitHub evaluates the
//! broken value regardless, so a green run is no evidence. Only a parse can
//! tell the two apart, which is why `RunnerSelection` keeps the raw string it
//! was given.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_norway::Value;

/// The directory holding this repository's own workflows.
pub(crate) const WORKFLOWS: &str = ".github/workflows";

/// The paid runner label this repository is entitled to name.
pub(crate) const UBICLOUD_LABEL: &str = "ubicloud-standard-2";

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

/// How a job chooses its runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RunnerSelection {
    /// A single literal label, such as `ubuntu-latest`.
    Literal(String),
    /// An expression, kept with the arms it can evaluate to.
    Expression {
        /// The declaration exactly as the document carried it.
        raw: String,
        /// Every label the expression can evaluate to.
        arms: Vec<String>,
    },
    /// No `runs-on` at all: the job calls a reusable workflow.
    Delegated,
}

impl RunnerSelection {
    /// Return every label this selection can put a job on.
    pub(crate) fn labels(&self) -> Vec<String> {
        match self {
            Self::Literal(label) => vec![label.clone()],
            Self::Expression { arms, .. } => arms.clone(),
            Self::Delegated => Vec::new(),
        }
    }

    /// Return whether this selection names a runner at all.
    pub(crate) const fn names_a_runner(&self) -> bool {
        !matches!(self, Self::Delegated)
    }

    /// Return the declaration as written, for the line-break check.
    pub(crate) fn raw(&self) -> &str {
        match self {
            Self::Literal(label) => label,
            Self::Expression { raw, .. } => raw,
            Self::Delegated => "",
        }
    }
}

/// One job, as the contracts need to see it.
#[derive(Debug, Clone)]
pub(crate) struct Job {
    /// The file the job is declared in.
    pub(crate) workflow: String,
    /// The job's identifier, which is what a required context is derived from.
    pub(crate) id: String,
    /// How the job chooses its runner.
    pub(crate) runs_on: RunnerSelection,
    /// The declared ceiling, or None when the job inherits GitHub's default.
    pub(crate) timeout_minutes: Option<u64>,
    /// The job's display name, when it sets one.
    pub(crate) name: Option<String>,
}

impl Job {
    /// Return whether every label this job can reach is GitHub-hosted.
    pub(crate) fn is_github_hosted(&self) -> bool {
        let labels = self.labels_in_use();
        !labels.is_empty() && labels.iter().all(|l| HOSTED_LABELS.contains(&l.as_str()))
    }

    /// Return the labels this job can reach, expression arms included.
    pub(crate) fn labels_in_use(&self) -> Vec<String> {
        self.runs_on.labels()
    }

    /// Return the job's coordinate, for a message that names the fault.
    pub(crate) fn coordinate(&self) -> String {
        format!("{}:{}", self.workflow, self.id)
    }

    /// Return the text a required check's context is derived from.
    ///
    /// GitHub uses the job's `name` when it declares one and its identifier
    /// otherwise, so a contract reading only the identifier would miss the
    /// shape that actually causes the fault: a display name interpolating the
    /// runner.
    pub(crate) fn context_source(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// One workflow file and the events it answers.
#[derive(Debug, Clone)]
pub(crate) struct Workflow {
    /// The events the workflow answers, which decide what its lanes owe.
    pub(crate) events: BTreeSet<String>,
    /// Every job the workflow declares.
    pub(crate) jobs: Vec<Job>,
}

impl Workflow {
    /// Return whether this workflow runs for a pull request.
    ///
    /// A fork's pull request cannot obtain a paid runner, so this is what
    /// decides whether a lane owes the fork fallback rather than a bare label.
    pub(crate) fn serves_pull_requests(&self) -> bool {
        self.events.contains("pull_request")
    }

    /// Return whether this workflow runs on a push or a tag.
    pub(crate) fn serves_pushes(&self) -> bool {
        self.events.contains("push")
    }

    /// Return whether this workflow is a trunk or tag lane.
    ///
    /// Two conditions, not one. A workflow answering a push is a paid lane
    /// only if it serves no pull request as well: a workflow declaring both
    /// triggers owes the fork fallback, and requiring it to name the paid
    /// label outright would contradict that. This repository has no such
    /// workflow today, which is exactly why the predicate has to say so now
    /// rather than when one is added.
    pub(crate) fn is_trunk_or_tag(&self) -> bool {
        self.serves_pushes() && !self.serves_pull_requests()
    }

    /// Return whether this workflow answers only manual or automation events.
    ///
    /// `pull_request_target` is here rather than with the pull-request lanes
    /// deliberately: it runs against the base repository with the base
    /// repository's secrets, so a fork cannot reach it and it is never a
    /// candidate for the fork fallback.
    pub(crate) fn is_api_bound(&self) -> bool {
        !self.serves_pull_requests() && !self.serves_pushes()
    }
}

/// Return the expression arms a `runs-on` can evaluate to.
///
/// The arms are the single-quoted literals in the expression. Reading them
/// rather than matching the whole expression against a pattern means a lane
/// that is correctly placed but merely wrapped differently still passes, while
/// a lane whose fallback names the wrong label does not.
pub(crate) fn arms_of(raw: &str) -> Vec<String> {
    // Splitting on the quote leaves the quoted runs at the odd positions,
    // which avoids slicing the string: `indexing_slicing` and `string_slice`
    // are both denied here, and clippy lints tests under `--all-targets`.
    //
    // Only complete runs count. An expression with an odd number of quotes
    // has a dangling one, and the text after it sits at an odd position like
    // any closed run, so a reader taking every odd position would report the
    // remainder as an arm. `'ubuntu-latest' || 'ubicloud-standard-2` would
    // then present two arms and satisfy the fork-fallback contract while
    // GitHub evaluated something else entirely. Found by
    // `a_dangling_quote_contributes_no_arm` rather than by reading this.
    //
    // Taking the segments in pairs is what tells the two apart. After the
    // leading segment, each closed run is followed by the text up to the next
    // quote, so a run with nothing following it was never closed. Expressed
    // as pairs rather than as half the quote count because `integer_division`
    // and `integer_division_remainder_used` are both denied here.
    let segments: Vec<&str> = raw.split('\'').collect();
    segments
        .get(1..)
        .unwrap_or_default()
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .filter_map(|pair| pair.first())
        .map(|arm| (*arm).to_owned())
        .collect()
}

/// Return the `runs-on` of one job.
fn selection_of(job: &Value) -> RunnerSelection {
    let Some(raw) = job.get("runs-on").and_then(Value::as_str) else {
        return RunnerSelection::Delegated;
    };
    if raw.contains("${{") {
        RunnerSelection::Expression {
            raw: raw.to_owned(),
            arms: arms_of(raw),
        }
    } else {
        RunnerSelection::Literal(raw.to_owned())
    }
}

/// Return the events a workflow answers.
///
/// `on` is looked up as a string and as the boolean it becomes under a YAML
/// 1.1 reader, because `on` is a boolean key in that schema and one of this
/// repository's workflows quotes the key while the others do not. A reader
/// finding neither would report every workflow as answering nothing, and every
/// placement assertion below would then pass vacuously.
pub(crate) fn events_of(document: &Value) -> BTreeSet<String> {
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
pub(crate) fn workflow_paths() -> Vec<PathBuf> {
    let directory = repository_root().join(WORKFLOWS);
    let mut paths: Vec<PathBuf> = fs::read_dir(&directory)
        .unwrap_or_else(|err| panic!("{} must be readable: {err}", directory.display()))
        // An entry that cannot be read is a fault, not an absence. Discarding
        // it would shrink the set every contract below iterates over, and a
        // contract that silently inspects four workflows where there are five
        // reports success for the one it never saw.
        .map(|entry| {
            entry.unwrap_or_else(|err| panic!("{} must list cleanly: {err}", directory.display()))
        })
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
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();
    let jobs = match document.get("jobs") {
        Some(Value::Mapping(map)) => map
            .iter()
            .filter_map(|(id, job)| id.as_str().map(|id| (id, job)))
            .map(|(id, job)| Job {
                workflow: file.clone(),
                id: id.to_owned(),
                runs_on: selection_of(job),
                timeout_minutes: job.get("timeout-minutes").and_then(Value::as_u64),
                name: job.get("name").and_then(Value::as_str).map(str::to_owned),
            })
            .collect(),
        _ => Vec::new(),
    };
    Workflow {
        events: events_of(&document),
        jobs,
    }
}

/// Return every workflow this repository owns, parsed.
pub(crate) fn workflows() -> Vec<Workflow> {
    workflow_paths().iter().map(|p| workflow_at(p)).collect()
}

/// Return every job of every workflow, with its workflow beside it.
pub(crate) fn jobs(workflows: &[Workflow]) -> Vec<(&Workflow, &Job)> {
    workflows
        .iter()
        .flat_map(|w| w.jobs.iter().map(move |j| (w, j)))
        .collect()
}

/// Return the labels registered with actionlint.
pub(crate) fn registered_labels() -> BTreeSet<String> {
    let path = repository_root().join(".github/actionlint.yaml");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} must be readable: {err}", path.display()));
    let document: Value = serde_norway::from_str(&text)
        .unwrap_or_else(|err| panic!("{} must be valid YAML: {err}", path.display()));
    match document
        .get("self-hosted-runner")
        .and_then(|r| r.get("labels"))
    {
        Some(Value::Sequence(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => BTreeSet::new(),
    }
}

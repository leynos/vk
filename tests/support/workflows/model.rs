//! The parsed shape of one workflow: its triggers, jobs and steps.
//!
//! Each type keeps the raw YAML node it was read from beside the fields the
//! contracts ask about by name, so a question the fields do not answer, such
//! as "where is this secret mentioned at all", is asked of the whole node
//! rather than of whichever keys a field happened to be written for.

use std::collections::BTreeSet;

use serde_norway::Value;

use super::expression::{Scope, Secret};
use super::runner::RunnerSelection;

/// A name and the value it is set to, both rendered as text.
pub(crate) type Pair = (String, String);

/// Return the value paired with `key`, when one is.
fn value_of<'a>(pairs: &'a [Pair], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

/// One step of one job, as the contracts need to see it.
#[derive(Debug, Clone)]
pub(crate) struct Step {
    /// The step's name, when it declares one.
    pub(crate) name: Option<String>,
    /// The action the step calls, when it calls one.
    pub(crate) uses: Option<String>,
    /// The shell the step runs, when it runs one.
    pub(crate) run: Option<String>,
    /// The step's `if` condition, when it declares one.
    pub(crate) condition: Option<String>,
    /// The inputs the step passes, flattened to text.
    pub(crate) with: Vec<Pair>,
    /// The environment the step declares, flattened to text.
    pub(crate) env: Vec<Pair>,
    /// The step as written.
    pub(crate) raw: Value,
}

impl Step {
    /// Return one input's value, when the step passes it.
    pub(crate) fn input(&self, key: &str) -> Option<&str> {
        value_of(&self.with, key)
    }

    /// Return one environment variable's value, when the step declares it.
    pub(crate) fn env_value(&self, key: &str) -> Option<&str> {
        value_of(&self.env, key)
    }

    /// Return every key path within the step that references `secret`.
    pub(crate) fn secret_sites(&self, secret: Secret) -> Vec<String> {
        secret.sites_in(&self.raw, Scope::Whole)
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
    /// The job's identifier, which is what a required context is derived
    /// from when the job sets no name.
    pub(crate) id: String,
    /// The job's display name, when it sets one.
    pub(crate) name: Option<String>,
    /// How the job chooses its runner.
    pub(crate) runs_on: RunnerSelection,
    /// The declared ceiling, or `None` when the job inherits GitHub's default.
    pub(crate) timeout_minutes: Option<u64>,
    /// The environment the job exports into every one of its steps.
    pub(crate) env: Vec<Pair>,
    /// The reusable workflow the job calls, when it calls one.
    pub(crate) calls: Option<String>,
    /// Whether the job hands every secret it can read to the workflow it
    /// calls, through `secrets: inherit`.
    pub(crate) inherits_secrets: bool,
    /// Whether a newer run of the job's concurrency group cancels this one.
    pub(crate) cancels_in_progress: bool,
    /// Every step the job declares.
    pub(crate) steps: Vec<Step>,
    /// The job as written.
    pub(crate) raw: Value,
}

impl Job {
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

    /// Return whether the job exports an environment variable named `key`.
    pub(crate) fn declares_env(&self, key: &str) -> bool {
        value_of(&self.env, key).is_some()
    }

    /// Return every key path outside the job's steps that references `secret`.
    ///
    /// The steps are left to [`Step::secret_sites`], so a site is reported
    /// once, against the narrowest scope that holds it.
    pub(crate) fn secret_sites(&self, secret: Secret) -> Vec<String> {
        secret.sites_in(&self.raw, Scope::Without("steps"))
    }
}

/// One workflow file and the events it answers.
#[derive(Debug, Clone)]
pub(crate) struct Workflow {
    /// The file name, for a message that names the fault.
    pub(crate) file: String,
    /// The events the workflow answers.
    pub(crate) events: BTreeSet<String>,
    /// The environment declared at workflow level.
    ///
    /// GitHub exports it into every step of every job, so a secret here has
    /// the widest reach a workflow can give it.
    pub(crate) env: Vec<Pair>,
    /// The branches a `push` trigger is filtered to, when it names any.
    pub(crate) push_branches: BTreeSet<String>,
    /// The tags a `push` trigger is filtered to, when it names any.
    pub(crate) push_tags: BTreeSet<String>,
    /// Whether a newer run of the workflow's concurrency group cancels this
    /// one.
    pub(crate) cancels_in_progress: bool,
    /// Every job the workflow declares.
    pub(crate) jobs: Vec<Job>,
    /// The workflow as written.
    pub(crate) raw: Value,
}

impl Workflow {
    /// Return whether this workflow is triggered by a pull request.
    ///
    /// `pull_request_target` counts. It runs against the base repository with
    /// the base repository's secrets, which is the more dangerous of the two
    /// places to put a token, not the safer one.
    pub(crate) fn serves_pull_requests(&self) -> bool {
        self.events.contains("pull_request") || self.events.contains("pull_request_target")
    }

    /// Return whether this workflow answers a push.
    pub(crate) fn serves_pushes(&self) -> bool {
        self.events.contains("push")
    }

    /// Return whether the workflow exports an environment variable named
    /// `key` into every job.
    pub(crate) fn declares_env(&self, key: &str) -> bool {
        value_of(&self.env, key).is_some()
    }

    /// Return every key path outside the workflow's jobs that references
    /// `secret`.
    pub(crate) fn secret_sites(&self, secret: Secret) -> Vec<String> {
        secret.sites_in(&self.raw, Scope::Without("jobs"))
    }
}

/// Return every job of every workflow, with its workflow beside it.
pub(crate) fn jobs<'a>(
    workflows: impl IntoIterator<Item = &'a Workflow>,
) -> Vec<(&'a Workflow, &'a Job)> {
    workflows
        .into_iter()
        .flat_map(|workflow| workflow.jobs.iter().map(move |job| (workflow, job)))
        .collect()
}

/// Return every step of every job, with its workflow and job beside it.
pub(crate) fn steps<'a>(
    workflows: impl IntoIterator<Item = &'a Workflow>,
) -> Vec<(&'a Workflow, &'a Job, &'a Step)> {
    jobs(workflows)
        .into_iter()
        .flat_map(|(workflow, job)| job.steps.iter().map(move |step| (workflow, job, step)))
        .collect()
}

//! What counts as contacting CodeScene, uploading to it, and publishing.
//!
//! The vocabulary the coverage contracts share. Each predicate reads a step
//! or workflow by shape, so the pull-request rules and the publisher rules
//! agree on what they are talking about.

use std::collections::BTreeSet;

use crate::reader::{Job, Secret, Step, Workflow};

/// The secret that authenticates a CodeScene upload, and the environment
/// variable the upload step holds it in.
pub(crate) const CODESCENE_TOKEN: &str = "CS_ACCESS_TOKEN";

/// The same secret, as the `secrets` context reads it.
pub(crate) const TOKEN_SECRET: Secret = Secret(CODESCENE_TOKEN);

/// The branch whose coverage CodeScene analyses, and so the only one that may
/// be published from.
pub(crate) const PUBLISHED_BRANCH: &str = "main";

/// The conjunct that keeps the upload from running without its token.
pub(crate) const TOKEN_GUARD: &str = "env.CS_ACCESS_TOKEN != ''";

/// The conjunct that keeps the upload from running on any ref but `main`.
///
/// The trigger filter alone does not do it: the publisher also answers
/// `workflow_dispatch`, which runs against whichever branch the dispatcher
/// picks.
pub(crate) const REF_GUARD: &str = "github.ref == 'refs/heads/main'";

/// The value the uploader's `access-token` input must read.
pub(crate) const TOKEN_INPUT: &str = "${{ env.CS_ACCESS_TOKEN }}";

/// The only key path within an upload step where the token may be read.
pub(crate) const TOKEN_SITE: &str = "env.CS_ACCESS_TOKEN";

/// What a step that reaches CodeScene asks it to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    /// Send coverage for an analysed branch.
    Upload,
    /// Gate on coverage, which is the pull-request mode this repository no
    /// longer runs.
    Check,
    /// Reach CodeScene without saying which, such as the action with no
    /// `mode`, or a bare request to its host.
    Unstated,
}

/// Return the operation a `cs-coverage` invocation in `script` performs.
///
/// A script invoking the tool more than once is an upload only when every
/// invocation is, so an upload cannot hide a check beside it.
fn command_operation(script: &str) -> Option<Operation> {
    if !script.contains("cs-coverage") {
        return None;
    }
    let words: Vec<&str> = script.split_whitespace().collect();
    let verbs: Vec<Operation> = words
        .windows(2)
        .filter(|pair| {
            pair.first()
                .is_some_and(|word| word.ends_with("cs-coverage"))
        })
        .map(|pair| match pair.get(1).copied() {
            Some("upload") => Operation::Upload,
            Some("check") => Operation::Check,
            _ => Operation::Unstated,
        })
        .collect();
    Some(if verbs.contains(&Operation::Check) {
        Operation::Check
    } else if !verbs.is_empty() && verbs.iter().all(|verb| *verb == Operation::Upload) {
        Operation::Upload
    } else {
        Operation::Unstated
    })
}

/// Return whether any text the step carries names CodeScene's host.
fn names_host(step: &Step) -> bool {
    let is_host = |text: &str| text.to_ascii_lowercase().contains("codescene.io");
    step.run.as_deref().is_some_and(is_host)
        || step
            .with
            .iter()
            .chain(&step.env)
            .any(|(_, value)| is_host(value))
}

/// Return what a step asks of CodeScene, or `None` when it never reaches it.
///
/// Three routes, in order: the CodeScene action, whose `mode` input says what
/// it does; a `cs-coverage` command, whose verb does; and any other mention
/// of CodeScene's host, such as a `curl`, which says nothing.
pub(crate) fn operation_of(step: &Step) -> Option<Operation> {
    let is_action = step
        .uses
        .as_deref()
        .is_some_and(|action| action.to_ascii_lowercase().contains("codescene"));
    if is_action {
        return Some(match step.input("mode") {
            Some("upload") => Operation::Upload,
            Some("check") => Operation::Check,
            _ => Operation::Unstated,
        });
    }
    step.run
        .as_deref()
        .and_then(command_operation)
        .or_else(|| names_host(step).then_some(Operation::Unstated))
}

/// Return whether a job hands its work to a CodeScene reusable workflow.
pub(crate) fn calls_codescene(job: &Job) -> bool {
    job.calls
        .as_deref()
        .is_some_and(|called| called.to_ascii_lowercase().contains("codescene"))
}

/// Return whether a step holds the token by any route.
///
/// Declaring it under its own name is one route; reading the secret into a
/// variable of any other name, an input, or a script is another.
pub(crate) fn holds_token(step: &Step) -> bool {
    step.env_value(CODESCENE_TOKEN).is_some() || !step.secret_sites(TOKEN_SECRET).is_empty()
}

/// Return whether this workflow is the coverage publisher.
///
/// Four conditions. It answers a push; it serves no pull request, since a
/// workflow declaring both would be required to upload and forbidden from
/// uploading at once; its push trigger is filtered to `main` and to nothing
/// else; and it names no tag filter, which would fire it for tags as well.
/// Equality rather than containment: `branches: [main, release]` contains
/// `main` and publishes from `release`.
pub(crate) fn is_publisher(workflow: &Workflow) -> bool {
    workflow.serves_pushes()
        && !workflow.serves_pull_requests()
        && workflow.push_branches == BTreeSet::from([PUBLISHED_BRANCH.to_owned()])
        && workflow.push_tags.is_empty()
}

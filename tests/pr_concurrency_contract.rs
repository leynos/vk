//! Every pull-request-startable workflow cancels superseded runs.
//!
//! Pushing twice to a pull request in quick succession leaves the first run
//! charging minutes for a result nobody will read. GitHub cancels it only when
//! the workflow declares a concurrency group and asks for it, so the fact is a
//! property of every workflow a pull request can start, not of any one job.
//!
//! The reader is a pure function of a workflow's text. The sweep reads this
//! repository's own workflows; the cases after it drive the reader with
//! synthetic sources, because a rule parametrized over files that already
//! conform passes whether or not it discriminates. Those synthetic cases are
//! what prove it rejects a deleted line, a literal `true` that would also
//! cancel the push to `main`, and a group keyed on the run identifier that
//! serializes nothing.
//!
//! The sweep also asserts that it found something. A contract over a filtered
//! list is satisfied by an empty list, so a reader that stopped recognizing the
//! trigger would report every workflow as conforming.

use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;

/// The trigger a pull request starts.
///
/// `pull_request_target` is deliberately excluded. It runs with the base
/// repository's token, and the workflow on it here merges; cancelling that
/// mid-write is not a saving.
const PULL_REQUEST_TRIGGER: &str = "pull_request";

/// The only accepted `cancel-in-progress` value.
///
/// A literal `true` would cancel a push to `main` or a dispatch sharing the
/// group, so the contract requires the guarded expression rather than merely a
/// truthy setting.
const CANCEL_IN_PROGRESS: &str = "${{ github.event_name == 'pull_request' }}";

/// A group keyed on the run identifier is unique per run, so it serializes
/// nothing and can never cancel a predecessor.
const RUN_ID_EXPRESSION: &str = "github.run_id";

/// The extensions GitHub accepts for a workflow document. A contract that
/// scanned one of them would leave the other unguarded.
const WORKFLOW_EXTENSIONS: [&str; 2] = ["yml", "yaml"];

fn workflow_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".github")
        .join("workflows")
}

/// Every workflow's file name and source text, in file-name order.
fn workflow_sources() -> Vec<(String, String)> {
    let directory = workflow_directory();
    let entries = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("{} could not be read: {error}", directory.display()));
    let mut sources: Vec<(String, String)> = entries
        .map(|entry| {
            entry
                .expect("workflow directory entry should be readable")
                .path()
        })
        .filter(|path| {
            path.extension().is_some_and(|extension| {
                WORKFLOW_EXTENSIONS
                    .iter()
                    .any(|accepted| extension.eq_ignore_ascii_case(accepted))
            })
        })
        .map(|path| {
            let name = path
                .file_name()
                .expect("a workflow path should have a file name")
                .to_string_lossy()
                .into_owned();
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{} could not be read: {error}", path.display()));
            (name, source)
        })
        .collect();
    sources.sort_by(|left, right| left.0.cmp(&right.0));
    assert!(
        !sources.is_empty(),
        "no workflow was found under {}, so every contract here would pass \
         having read nothing",
        directory.display()
    );
    sources
}

/// The remainder of a line that opens the named top-level key, if it does.
///
/// The key is matched bare and quoted, because YAML 1.1 folds an unquoted `on`
/// to the boolean `true` and several repositories write `"on":` to avoid that.
fn opening_remainder<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let bare = format!("{key}:");
    let quoted = format!("\"{key}\":");
    line.strip_prefix(&bare)
        .or_else(|| line.strip_prefix(&quoted))
        .map(str::trim)
}

/// Whether a line opens a new top-level block, ending the previous one.
///
/// Blank lines and comments belong to the block they sit in: a comment at
/// column zero between two settings would otherwise truncate the block above
/// it.
fn opens_a_top_level_block(line: &str) -> bool {
    !line.is_empty() && !line.starts_with([' ', '\t']) && !line.starts_with('#')
}

/// The text of one top-level block, as `(remainder of the key line, body)`.
///
/// A workflow's top-level keys sit at column zero, so the block runs from the
/// key line to the next line that opens one. Reading it this way rather than
/// with a YAML parser keeps this contract free of a dependency the crate does
/// not otherwise carry; the synthetic cases below are what hold the reader
/// honest.
fn top_level_block<'a>(source: &'a str, key: &str) -> Option<(&'a str, String)> {
    let mut lines = source.lines();
    let inline = lines.find_map(|line| opening_remainder(line, key))?;
    let body = lines
        .take_while(|line| !opens_a_top_level_block(line))
        .fold(String::new(), |mut body, line| {
            body.push_str(line);
            body.push('\n');
            body
        });
    Some((inline, body))
}

/// The event names a workflow declares, in the three shapes GitHub accepts.
///
/// A mapping puts each event on its own indented line; a sequence and a bare
/// scalar put them on the key line. A reader that handled one shape would
/// sweep an incomplete set.
fn trigger_names(source: &str) -> Vec<String> {
    let Some((inline, body)) = top_level_block(source, "on") else {
        return Vec::new();
    };
    let inline_names = inline
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    let mapped_names = body.lines().filter_map(|line| {
        let trimmed = line.trim_start();
        if line == trimmed || trimmed.starts_with('#') {
            return None;
        }
        let indent = line.len() - trimmed.len();
        if indent != 2 {
            return None;
        }
        Some(
            trimmed
                .split(':')
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned(),
        )
    });
    inline_names.chain(mapped_names).collect()
}

/// Whether a pull request can start this workflow.
fn is_pull_request_startable(source: &str) -> bool {
    trigger_names(source)
        .iter()
        .any(|name| name == PULL_REQUEST_TRIGGER)
}

/// One setting of a `concurrency:` block, as written.
#[derive(Debug, Default, PartialEq, Eq)]
struct Concurrency {
    group: Option<String>,
    cancel_in_progress: Option<String>,
}

/// The `concurrency:` block of a workflow, or `None` when it declares none.
fn concurrency(source: &str) -> Option<Concurrency> {
    let (_, body) = top_level_block(source, "concurrency")?;
    let setting = |key: &str| {
        body.lines().find_map(|line| {
            let trimmed = line.trim_start();
            trimmed
                .strip_prefix(key)
                .and_then(|rest| rest.strip_prefix(':'))
                .map(|value| value.trim().to_owned())
        })
    };
    Some(Concurrency {
        group: setting("group"),
        cancel_in_progress: setting("cancel-in-progress"),
    })
}

/// Every way a workflow's text fails the contract, named.
fn violations(source: &str) -> Vec<String> {
    let Some(settings) = concurrency(source) else {
        return vec!["declares no concurrency: block".to_owned()];
    };
    let mut found = Vec::new();
    match settings.group.as_deref() {
        None | Some("") => found.push("declares no concurrency group".to_owned()),
        Some(group) if group.contains(RUN_ID_EXPRESSION) => {
            found.push(format!("keys its concurrency group on {RUN_ID_EXPRESSION}"));
        }
        Some(_) => {}
    }
    match settings.cancel_in_progress.as_deref() {
        None => found.push("sets no cancel-in-progress".to_owned()),
        Some(value) if value == CANCEL_IN_PROGRESS => {}
        Some(value) => found.push(format!(
            "sets cancel-in-progress to {value} and not {CANCEL_IN_PROGRESS}"
        )),
    }
    found
}

/// The workflows a pull request can start, in file-name order.
fn pull_request_workflows() -> Vec<(String, String)> {
    workflow_sources()
        .into_iter()
        .filter(|(_, source)| is_pull_request_startable(source))
        .collect()
}

#[test]
fn the_repository_has_pull_request_workflows() {
    assert!(
        !pull_request_workflows().is_empty(),
        "no workflow was read as pull-request-startable, so the sweep below \
         asserts nothing; the trigger reader or the workflow directory moved"
    );
}

#[test]
fn every_pull_request_workflow_cancels_superseded_runs() {
    for (name, source) in pull_request_workflows() {
        let found = violations(&source);
        assert!(found.is_empty(), "{name} {}", found.join("; "));
    }
}

/// A workflow written the way this repository deploys it.
fn conforming(cancel: &str) -> String {
    format!(
        "name: CI\n\
         on:\n  pull_request:\n    branches: [main]\n\n\
         concurrency:\n  \
         group: ${{{{ github.workflow }}}}-${{{{ github.event.pull_request.number }}}}\n  \
         cancel-in-progress: {cancel}\n\n\
         jobs:\n  build-test:\n    runs-on: ubuntu-latest\n"
    )
}

#[test]
fn the_deployed_shape_reports_no_violation() {
    let source = conforming(CANCEL_IN_PROGRESS);
    assert!(
        violations(&source).is_empty(),
        "the deployed shape must report no violation, or the rejection cases \
         below would pass against a reader that refused everything: {:?}",
        violations(&source)
    );
}

#[rstest]
#[case::cancel_line_removed(
    conforming(CANCEL_IN_PROGRESS).replace(
        "  cancel-in-progress: ${{ github.event_name == 'pull_request' }}\n",
        "",
    ),
    "no cancel-in-progress"
)]
#[case::literal_true(conforming("true"), "cancel-in-progress to true")]
#[case::quoted_true(conforming("'true'"), "cancel-in-progress to 'true'")]
#[case::run_id_group(
    conforming(CANCEL_IN_PROGRESS).replace(
        "${{ github.workflow }}-${{ github.event.pull_request.number }}",
        "ci-${{ github.run_id }}",
    ),
    "github.run_id"
)]
#[case::no_concurrency_block("name: CI\non:\n  pull_request:\n\njobs: {}\n".to_owned(), "no concurrency")]
fn a_non_conforming_workflow_is_rejected(#[case] source: String, #[case] fragment: &str) {
    let found = violations(&source);
    assert!(
        found.iter().any(|violation| violation.contains(fragment)),
        "{fragment:?} should have been reported, got {found:?}"
    );
}

#[rstest]
#[case::mapping("name: CI\non:\n  push:\n  pull_request:\n")]
#[case::quoted_key("name: CI\n\"on\":\n  pull_request:\n    types: [opened]\n")]
#[case::sequence("name: CI\non: [push, pull_request]\n")]
#[case::bare_scalar("name: CI\non: pull_request\n")]
fn the_trigger_reader_covers_every_accepted_shape(#[case] source: &str) {
    assert!(
        is_pull_request_startable(source),
        "{source:?} declares the pull_request trigger and must read as \
         pull-request-startable, or the sweep skips it"
    );
}

#[rstest]
#[case::push_only("name: CI\non:\n  push:\n    branches: [main]\n")]
#[case::target("name: CI\non:\n  pull_request_target:\n    types: [opened]\n")]
#[case::target_inline("name: CI\non: [pull_request_target]\n")]
#[case::schedule("name: CI\non:\n  schedule:\n    - cron: '0 3 * * *'\n")]
fn a_workflow_no_pull_request_starts_is_out_of_scope(#[case] source: &str) {
    assert!(
        !is_pull_request_startable(source),
        "{source:?} declares no pull_request trigger and must stay out of \
         scope, or the rule widens past what was approved"
    );
}

#[test]
fn a_nested_key_is_not_read_as_a_trigger() {
    // `pull_request` appears at four spaces here, under another event's
    // settings. A reader that searched the block for the word would count it.
    let source = "name: CI\non:\n  workflow_run:\n    workflows: [pull_request]\n";
    assert!(
        !is_pull_request_startable(source),
        "an event name nested under another event is not a trigger"
    );
}

#[test]
fn a_block_ends_at_the_next_top_level_key() {
    let source = "name: CI\nconcurrency:\n  group: ci\njobs:\n  cancel-in-progress: true\n";
    assert_eq!(
        concurrency(source),
        Some(Concurrency {
            group: Some("ci".to_owned()),
            cancel_in_progress: None,
        }),
        "a setting under a later top-level key must not be read as this \
         block's, or a workflow could satisfy the contract by accident"
    );
}

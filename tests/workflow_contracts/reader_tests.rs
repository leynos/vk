//! The workflow reader, tested from text and from a temporary directory.
//!
//! The repository's own workflows exercise only the shapes they happen to be
//! written in. These cases feed the reader every shape a workflow may take,
//! and every way reading one can fail, so a contract's silence over the real
//! files means the files are clean rather than that the reader looked away.

use std::collections::BTreeSet;

use cap_std::ambient_authority;
use cap_std::fs_utf8::Dir;
use rstest::rstest;

use crate::codescene::TOKEN_SECRET;
use crate::reader::{
    Condition, Scope, Secret, WorkflowError, load, local_call_target, parse_workflow,
    pull_request_closure,
};

/// Return a set of names, for comparing against a parsed one.
fn names(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[rstest]
#[case::scalar("on: pull_request\n", &["pull_request"])]
#[case::sequence("on: [push, pull_request]\n", &["push", "pull_request"])]
#[case::mapping("on:\n  push:\n  workflow_dispatch:\n", &["push", "workflow_dispatch"])]
#[case::quoted_key("'on':\n  push:\n    tags: ['v*']\n", &["push"])]
#[case::boolean_key("true: [push]\n", &["push"])]
#[case::absent("name: nothing\n", &[])]
fn triggers_are_read_in_every_shape(
    #[case] text: &str,
    #[case] expected: &[&str],
) -> Result<(), WorkflowError> {
    assert_eq!(parse_workflow("x.yml", text)?.events, names(expected));
    Ok(())
}

#[rstest]
#[case::sequence("on:\n  push:\n    branches: [main, release]\n", &["main", "release"])]
#[case::scalar("on:\n  push:\n    branches: main\n", &["main"])]
#[case::unfiltered("on:\n  push:\n", &[])]
#[case::list_form("on: [push]\n", &[])]
fn push_branch_filters_are_read_in_every_shape(
    #[case] text: &str,
    #[case] expected: &[&str],
) -> Result<(), WorkflowError> {
    assert_eq!(
        parse_workflow("x.yml", text)?.push_branches,
        names(expected)
    );
    Ok(())
}

#[rstest]
#[case::bad_syntax("on: [push\n")]
#[case::duplicate_key(
    "jobs:\n  a:\n    runs-on: ubuntu-latest\n    runs-on: ubicloud-standard-2\n"
)]
fn unparseable_text_is_refused(#[case] text: &str) {
    assert!(matches!(
        parse_workflow("x.yml", text),
        Err(WorkflowError::Parse { .. })
    ));
}

#[test]
fn a_document_that_is_not_a_mapping_is_refused() {
    assert!(matches!(
        parse_workflow("x.yml", "- not a mapping\n"),
        Err(WorkflowError::Shape { .. })
    ));
}

#[test]
fn the_source_text_is_kept_exactly_comments_included() -> Result<(), WorkflowError> {
    let text = "# a comment the parse discards\non: push # and another\njobs: {}\n";
    assert_eq!(parse_workflow("x.yml", text)?.text, text);
    Ok(())
}

#[test]
fn unquoted_scalars_are_rendered_rather_than_dropped() -> Result<(), WorkflowError> {
    let text = "jobs:\n  a:\n    steps:\n      - with: {mode: upload, retries: 3, quiet: true}\n";
    let workflow = parse_workflow("x.yml", text)?;
    let step = workflow
        .jobs
        .first()
        .and_then(|job| job.steps.first())
        .expect("the step is parsed");
    assert_eq!(step.input("mode"), Some("upload"));
    assert_eq!(step.input("retries"), Some("3"));
    assert_eq!(step.input("quiet"), Some("true"));
    Ok(())
}

#[rstest]
#[case::absent("", false)]
#[case::scalar_group("concurrency: group\n", false)]
#[case::literal_false("concurrency: {group: g, cancel-in-progress: false}\n", false)]
#[case::quoted_false("concurrency: {group: g, cancel-in-progress: 'false'}\n", false)]
#[case::literal_true("concurrency: {group: g, cancel-in-progress: true}\n", true)]
#[case::expression("concurrency: {group: g, cancel-in-progress: '${{ x }}'}\n", true)]
fn cancel_in_progress_is_read_conservatively(
    #[case] block: &str,
    #[case] expected: bool,
) -> Result<(), WorkflowError> {
    let workflow = parse_workflow("x.yml", &format!("on: push\n{block}"))?;
    assert_eq!(workflow.cancels_in_progress, expected);
    Ok(())
}

#[rstest]
#[case::dotted("${{ secrets.CS_ACCESS_TOKEN }}", true)]
#[case::lower_case("${{ secrets.cs_access_token }}", true)]
#[case::indexed_single("${{ secrets['CS_ACCESS_TOKEN'] }}", true)]
#[case::indexed_double("${{ secrets[\"CS_ACCESS_TOKEN\"] }}", true)]
#[case::spaced("${{ secrets . CS_ACCESS_TOKEN }}", true)]
#[case::every_secret("${{ toJSON(secrets) }}", true)]
#[case::in_a_script("curl -H \"x: ${{ secrets.CS_ACCESS_TOKEN }}\" host", true)]
#[case::longer_name("${{ secrets.CS_ACCESS_TOKEN_OLD }}", false)]
#[case::environment("${{ env.CS_ACCESS_TOKEN }}", false)]
fn secret_references_are_read_in_every_spelling(#[case] text: &str, #[case] expected: bool) {
    assert_eq!(TOKEN_SECRET.is_read_by(text), expected);
}

#[test]
fn secret_sites_are_reported_against_the_narrowest_scope() -> Result<(), WorkflowError> {
    let text = concat!(
        "env: {A: '${{ secrets.T }}'}\n",
        "jobs:\n  j:\n    env: {B: '${{ secrets.T }}'}\n",
        "    steps:\n      - env: {C: '${{ secrets.T }}'}\n        run: echo\n",
    );
    let workflow = parse_workflow("x.yml", text)?;
    let job = workflow.jobs.first().expect("the job is parsed");
    let step = job.steps.first().expect("the step is parsed");
    let secret = Secret("T");
    assert_eq!(workflow.secret_sites(secret), vec!["env.A".to_owned()]);
    assert_eq!(job.secret_sites(secret), vec!["env.B".to_owned()]);
    assert_eq!(step.secret_sites(secret), vec!["env.C".to_owned()]);
    Ok(())
}

#[test]
fn secret_sites_inside_a_sequence_are_reported_by_index() -> Result<(), serde_norway::Error> {
    // A sequence is walked item by item, so a secret passed as the second of
    // a job's service options is found, and its path says which item it is.
    let node: serde_norway::Value =
        serde_norway::from_str("options: ['--rm', '${{ secrets.T }}']\n")?;
    assert_eq!(
        Secret("T").sites_in(&node, Scope::Whole),
        vec!["options[1]".to_owned()]
    );
    Ok(())
}

#[rstest]
#[case::plain("a && b", Ok(vec!["a", "b"]))]
#[case::wrapped("${{ a  &&  b }}", Ok(vec!["a", "b"]))]
#[case::quoted_operator("a && b == 'x || y'", Ok(vec!["a", "b == 'x || y'"]))]
#[case::disjunction("a && b || c", Err(()))]
fn conditions_split_on_conjunction_and_refuse_disjunction(
    #[case] condition: &str,
    #[case] expected: Result<Vec<&str>, ()>,
) {
    let parts = Condition(condition).conjuncts().map_err(|_| ());
    let expected = expected.map(|parts| parts.into_iter().map(str::to_owned).collect::<Vec<_>>());
    assert_eq!(parts, expected);
}

#[rstest]
#[case::dotted("./.github/workflows/probe.yml", Some("probe.yml"))]
#[case::bare(".github/workflows/probe.yml", Some("probe.yml"))]
#[case::remote("leynos/shared-actions/.github/workflows/x.yml@abc", None)]
fn local_calls_are_recognized_by_shape(#[case] uses: &str, #[case] expected: Option<&str>) {
    assert_eq!(local_call_target(uses), expected);
}

#[test]
fn the_closure_follows_local_calls_transitively() -> Result<(), WorkflowError> {
    let workflows = [
        parse_workflow(
            "ci.yml",
            "on: pull_request\njobs:\n  a:\n    uses: ./.github/workflows/b.yml\n",
        )?,
        parse_workflow(
            "b.yml",
            "on: workflow_call\njobs:\n  b:\n    uses: .github/workflows/c.yml\n",
        )?,
        parse_workflow("c.yml", "on: workflow_call\njobs: {}\n")?,
        parse_workflow("main.yml", "on: push\njobs: {}\n")?,
    ];
    let reached: BTreeSet<&str> = pull_request_closure(&workflows)?
        .into_iter()
        .map(|workflow| workflow.file.as_str())
        .collect();
    assert_eq!(reached, BTreeSet::from(["ci.yml", "b.yml", "c.yml"]));
    Ok(())
}

#[test]
fn a_call_to_a_missing_workflow_is_an_error() -> Result<(), WorkflowError> {
    let workflows = [parse_workflow(
        "ci.yml",
        "on: pull_request\njobs:\n  a:\n    uses: ./.github/workflows/gone.yml\n",
    )?];
    assert!(matches!(
        pull_request_closure(&workflows),
        Err(WorkflowError::UnresolvedCall { .. })
    ));
    Ok(())
}

/// Return a temporary directory and a capability for it.
fn scratch() -> std::io::Result<(tempfile::TempDir, Dir)> {
    let holder = tempfile::tempdir()?;
    let path = holder
        .path()
        .to_str()
        .ok_or_else(|| std::io::Error::other("the temporary path is not UTF-8"))?
        .to_owned();
    let directory = Dir::open_ambient_dir(path, ambient_authority())?;
    Ok((holder, directory))
}

#[test]
fn the_loader_reads_every_extension_in_any_case() -> std::io::Result<()> {
    let (_holder, directory) = scratch()?;
    directory.write("a.yml", "on: push\n")?;
    directory.write("b.YAML", "on: pull_request\n")?;
    directory.write("notes.txt", "not a workflow")?;
    let files: Vec<String> = load(&directory)
        .map_err(std::io::Error::other)?
        .into_iter()
        .map(|workflow| workflow.file)
        .collect();
    assert_eq!(files, vec!["a.yml".to_owned(), "b.YAML".to_owned()]);
    Ok(())
}

#[test]
fn the_loader_refuses_an_empty_directory() -> std::io::Result<()> {
    let (_holder, directory) = scratch()?;
    assert!(matches!(load(&directory), Err(WorkflowError::Empty)));
    Ok(())
}

#[test]
fn the_loader_reports_an_unreadable_workflow() -> std::io::Result<()> {
    let (_holder, directory) = scratch()?;
    directory.create_dir("folder.yml")?;
    assert!(matches!(load(&directory), Err(WorkflowError::Read { .. })));
    Ok(())
}

#[test]
fn the_loader_reports_invalid_yaml_by_file() -> std::io::Result<()> {
    let (_holder, directory) = scratch()?;
    directory.write("broken.yml", "on: [push\n")?;
    match load(&directory) {
        Err(WorkflowError::Parse { file, .. }) => assert_eq!(file, "broken.yml"),
        other => panic!("expected a parse error naming the file, got {other:?}"),
    }
    Ok(())
}

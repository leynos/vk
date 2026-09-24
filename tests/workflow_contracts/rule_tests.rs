//! The coverage rules, tested against constructed workflows.
//!
//! Each case builds the smallest workflow set that breaks one rule in one way,
//! and checks the rule reports it; the unbroken baseline checks it reports
//! nothing. Run against the repository alone, a rule is only ever seen
//! passing, which is also what a rule that checks nothing does.

use rstest::rstest;

use crate::codescene::is_publisher;
use crate::publisher::{concurrency_faults, guard_faults, mode_faults, token_faults};
use crate::pull_request_lanes::{ratchet_faults, reach_faults};
use crate::reader::{Workflow, WorkflowError, parse_workflow};

/// The pull-request lane every constructed set starts from.
const LANE: &str = concat!(
    "on: pull_request\n",
    "jobs:\n  build:\n    steps:\n",
    "      - uses: leynos/shared-actions/.github/actions/generate-coverage@x\n",
    "        with: {with-ratchet: 'true'}\n",
);

/// The availability check and the upload step, as the publisher carries them.
const UPLOAD: &str = concat!(
    "      - name: Check\n",
    "        id: codescene_token\n",
    "        run: echo \"available=${{ secrets.CS_ACCESS_TOKEN != '' }}\" >> \"$GITHUB_OUTPUT\"\n",
    "      - name: Upload\n",
    "        if: steps.codescene_token.outputs.available == 'true' && github.ref == 'refs/heads/main'\n",
    "        uses: leynos/shared-actions/.github/actions/upload-codescene-coverage@x\n",
    "        with: {mode: upload, access-token: '${{ secrets.CS_ACCESS_TOKEN }}'}\n",
);

/// The upload step as the publisher carried it before the token left `env`.
const RETIRED_UPLOAD: &str = concat!(
    "      - name: Upload\n",
    "        env: {CS_ACCESS_TOKEN: '${{ secrets.CS_ACCESS_TOKEN }}'}\n",
    "        if: env.CS_ACCESS_TOKEN != '' && github.ref == 'refs/heads/main'\n",
    "        uses: leynos/shared-actions/.github/actions/upload-codescene-coverage@x\n",
    "        with: {mode: upload, access-token: '${{ env.CS_ACCESS_TOKEN }}'}\n",
);

/// Return a publisher whose one job runs `steps`, under `preamble`.
fn publisher(preamble: &str, steps: &str) -> String {
    format!(
        "on:\n  push:\n    branches: [main]\n  workflow_dispatch:\n{preamble}\
         jobs:\n  upload:\n    steps:\n{steps}"
    )
}

/// Return the lane and a publisher built from `preamble` and `steps`, parsed.
fn set(preamble: &str, steps: &str) -> Result<Vec<Workflow>, WorkflowError> {
    Ok(vec![
        parse_workflow("ci.yml", LANE)?,
        parse_workflow("main.yml", &publisher(preamble, steps))?,
    ])
}

#[test]
fn the_baseline_breaks_no_rule() -> Result<(), WorkflowError> {
    let workflows = set("", UPLOAD)?;
    assert_eq!(reach_faults(&workflows)?, Vec::<String>::new());
    assert_eq!(ratchet_faults(&workflows)?, (1, Vec::new()));
    assert_eq!(mode_faults(&workflows), Vec::<String>::new());
    assert_eq!(token_faults(&workflows), Vec::<String>::new());
    assert_eq!(guard_faults(&workflows), Vec::<String>::new());
    assert_eq!(concurrency_faults(&workflows), Vec::<String>::new());
    Ok(())
}

#[rstest]
#[case::main_only("on:\n  push:\n    branches: [main]\n", true)]
#[case::main_and_release("on:\n  push:\n    branches: [main, release]\n", false)]
#[case::release_only("on:\n  push:\n    branches: [release]\n", false)]
#[case::unfiltered_push("on: push\n", false)]
#[case::tag_push("on:\n  push:\n    tags: ['v*']\n", false)]
#[case::main_and_tags("on:\n  push:\n    branches: [main]\n    tags: ['v*']\n", false)]
#[case::push_and_pull_request("on:\n  push:\n    branches: [main]\n  pull_request:\n", false)]
fn the_publisher_is_a_push_to_main_alone(
    #[case] text: &str,
    #[case] expected: bool,
) -> Result<(), WorkflowError> {
    assert_eq!(is_publisher(&parse_workflow("x.yml", text)?), expected);
    Ok(())
}

#[rstest]
#[case::dotted("./.github/workflows/probe.yml")]
#[case::bare(".github/workflows/probe.yml")]
fn a_reusable_workflow_called_from_a_lane_is_held_to_its_rules(
    #[case] call: &str,
) -> Result<(), WorkflowError> {
    // The probe: a workflow declaring only `workflow_call`, called from a
    // pull-request job with `secrets: inherit`, and curling CodeScene's API
    // with the inherited token.
    let lane =
        format!("on: pull_request\njobs:\n  call:\n    uses: {call}\n    secrets: inherit\n");
    let probe = concat!(
        "on: workflow_call\njobs:\n  probe:\n    steps:\n",
        "      - run: 'curl -H \"Authorization: ${{ secrets.CS_ACCESS_TOKEN }}\" ",
        "https://api.codescene.io/v2/projects'\n",
    );
    let workflows = [
        parse_workflow("ci.yml", &lane)?,
        parse_workflow("probe.yml", probe)?,
    ];
    let faults = reach_faults(&workflows)?;
    assert!(
        faults
            .iter()
            .any(|fault| fault.contains("secrets: inherit")),
        "{faults:?}"
    );
    assert!(
        faults
            .iter()
            .any(|fault| fault.contains("probe.yml:probe step")),
        "{faults:?}"
    );
    Ok(())
}

#[test]
fn a_lane_reaching_codescene_by_host_alone_is_refused() -> Result<(), WorkflowError> {
    // No action, no `cs-coverage`, no token: only the host. The contact is
    // the fault, whatever the request carries.
    let lane = format!("{LANE}      - run: curl https://api.codescene.io/v2/projects\n");
    let faults = reach_faults(&[parse_workflow("ci.yml", &lane)?])?;
    assert_eq!(faults.len(), 1, "{faults:?}");
    assert!(
        faults
            .iter()
            .all(|fault| fault.contains("contacts CodeScene")),
        "{faults:?}"
    );
    Ok(())
}

#[test]
fn a_lane_reading_the_token_under_another_name_is_refused() -> Result<(), WorkflowError> {
    let lane = format!("{LANE}        env: {{OTHER: '${{{{ secrets.cs_access_token }}}}'}}\n");
    let workflows = [parse_workflow("ci.yml", &lane)?];
    assert_eq!(reach_faults(&workflows)?.len(), 1);
    Ok(())
}

#[rstest]
#[case::check_command("      - run: cs-coverage check lcov.info\n")]
#[case::check_action(
    "      - uses: leynos/shared-actions/.github/actions/upload-codescene-coverage@x\n        with: {mode: check}\n"
)]
#[case::implicit_mode(
    "      - uses: leynos/shared-actions/.github/actions/upload-codescene-coverage@x\n"
)]
#[case::check_beside_upload("      - run: cs-coverage upload a && cs-coverage check b\n")]
fn a_publisher_step_that_does_not_upload_is_refused(
    #[case] step: &str,
) -> Result<(), WorkflowError> {
    assert_eq!(mode_faults(&set("", step)?).len(), 1);
    Ok(())
}

#[test]
fn a_shell_upload_cannot_hold_the_token() -> Result<(), WorkflowError> {
    // A shell upload can take the token only through `env` or its script, and
    // neither is allowed; without it, the upload has nothing to send.
    let bound = concat!(
        "      - env: {CS_ACCESS_TOKEN: '${{ secrets.CS_ACCESS_TOKEN }}'}\n",
        "        if: env.CS_ACCESS_TOKEN != '' && github.ref == 'refs/heads/main'\n",
        "        run: cs-coverage upload lcov.info\n",
    );
    let workflows = set("", bound)?;
    assert_eq!(mode_faults(&workflows), Vec::<String>::new());
    assert_eq!(token_faults(&workflows).len(), 1);
    assert_eq!(guard_faults(&workflows).len(), 1);

    let tokenless = "      - run: cs-coverage upload lcov.info\n";
    assert_eq!(token_faults(&set("", tokenless)?).len(), 1);
    Ok(())
}

#[rstest]
#[case::dispatch_from_anywhere(
    "&& github.ref == 'refs/heads/main'",
    "&& github.ref == 'refs/heads/main' || github.event_name == 'workflow_dispatch'"
)]
#[case::ref_guard_dropped(" && github.ref == 'refs/heads/main'", "")]
#[case::token_guard_dropped("steps.codescene_token.outputs.available == 'true' && ", "")]
#[case::credential_elsewhere("access-token: '${{ secrets.CS_ACCESS_TOKEN }}'", "access-token: x")]
#[case::credential_through_env(
    "access-token: '${{ secrets.CS_ACCESS_TOKEN }}'",
    "access-token: '${{ env.CS_ACCESS_TOKEN }}'"
)]
#[case::check_deleted(
    "      - name: Check\n        id: codescene_token\n        run: echo \"available=${{ secrets.CS_ACCESS_TOKEN != '' }}\" >> \"$GITHUB_OUTPUT\"\n",
    ""
)]
#[case::check_conditioned(
    "        id: codescene_token\n",
    "        id: codescene_token\n        if: github.ref == 'refs/heads/main'\n"
)]
#[case::check_command_changed(
    "run: echo \"available=${{ secrets.CS_ACCESS_TOKEN != '' }}\"",
    "run: echo \"available=true\""
)]
#[case::check_renamed("id: codescene_token", "id: other")]
fn a_weakened_upload_guard_is_refused(
    #[case] from: &str,
    #[case] to: &str,
) -> Result<(), WorkflowError> {
    assert_eq!(
        guard_faults(&set("", &UPLOAD.replacen(from, to, 1))?).len(),
        1
    );
    Ok(())
}

#[test]
fn the_retired_env_shape_is_refused() -> Result<(), WorkflowError> {
    // The token bound in the upload step's `env`, the guard reading it there,
    // and the input reading it back: the composite uploader's nested steps
    // inherit that `env`, so every one of them held the token.
    let workflows = set("", RETIRED_UPLOAD)?;
    assert_eq!(token_faults(&workflows).len(), 1);
    assert_eq!(guard_faults(&workflows).len(), 1);
    Ok(())
}

#[rstest]
#[case::upload_step(
    "      - name: Upload\n",
    "      - name: Upload\n        env: {CS_ACCESS_TOKEN: '${{ secrets.CS_ACCESS_TOKEN }}'}\n"
)]
#[case::check_step(
    "        id: codescene_token\n",
    "        id: codescene_token\n        env: {T: '${{ secrets.cs_access_token }}'}\n"
)]
#[case::build_step(
    "      - name: Check\n",
    "      - run: make\n        env: {T: '${{ secrets.CS_ACCESS_TOKEN }}'}\n      - name: Check\n"
)]
#[case::name_only(
    "      - name: Upload\n",
    "      - name: Upload\n        env: {CS_ACCESS_TOKEN: '${{ env.OTHER }}'}\n"
)]
fn a_token_in_any_step_env_is_refused(
    #[case] from: &str,
    #[case] to: &str,
) -> Result<(), WorkflowError> {
    assert_eq!(
        token_faults(&set("", &UPLOAD.replacen(from, to, 1))?).len(),
        1
    );
    Ok(())
}

#[test]
fn a_token_moved_off_the_upload_input_is_refused() -> Result<(), WorkflowError> {
    // Deleting the token satisfies every prohibition while the upload has
    // nothing to send: the rule must refuse the upload that lost it.
    let steps = UPLOAD.replacen(
        "access-token: '${{ secrets.CS_ACCESS_TOKEN }}'",
        "access-token: x",
        1,
    );
    assert_eq!(token_faults(&set("", &steps)?).len(), 1);
    Ok(())
}

#[test]
fn a_token_at_workflow_level_is_refused() -> Result<(), WorkflowError> {
    let preamble = "env: {T: '${{ secrets.CS_ACCESS_TOKEN }}'}\n";
    assert_eq!(token_faults(&set(preamble, UPLOAD)?).len(), 1);
    Ok(())
}

#[test]
fn a_publisher_that_cancels_in_progress_is_refused() -> Result<(), WorkflowError> {
    let queued = "concurrency: {group: coverage, cancel-in-progress: false}\n";
    let cancelling = "concurrency: {group: coverage, cancel-in-progress: true}\n";
    assert_eq!(
        concurrency_faults(&set(queued, UPLOAD)?),
        Vec::<String>::new()
    );
    assert_eq!(concurrency_faults(&set(cancelling, UPLOAD)?).len(), 1);
    Ok(())
}

#[test]
fn an_unratcheted_lane_is_refused() -> Result<(), WorkflowError> {
    let workflows = [parse_workflow(
        "ci.yml",
        &LANE.replacen("'true'", "'false'", 1),
    )?];
    assert_eq!(ratchet_faults(&workflows)?.1.len(), 1);
    Ok(())
}

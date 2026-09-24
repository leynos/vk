//! Properties of the publisher's token and guard rules over generated jobs.
//!
//! Each case builds one publisher job from unrelated steps, the availability
//! check and the upload in any order, and the token placed in at most one
//! `env` scope. The rules must accept exactly the cases where the check runs
//! before the upload and no `env` holds the token, and refuse every other,
//! whichever unrelated steps surround them.

use proptest::prelude::*;

use crate::publisher::{guard_faults, token_faults};
use crate::reader::parse_workflow;

/// The availability check, as the publisher carries it.
const CHECK: &str = concat!(
    "      - id: codescene_token\n",
    "        run: echo \"available=${{ secrets.CS_ACCESS_TOKEN != '' }}\" >> \"$GITHUB_OUTPUT\"\n",
);

/// The upload step, as the publisher carries it.
const UPLOAD: &str = concat!(
    "      - name: Upload\n",
    "        if: steps.codescene_token.outputs.available == 'true' && github.ref == 'refs/heads/main'\n",
    "        uses: leynos/shared-actions/.github/actions/upload-codescene-coverage@x\n",
    "        with: {mode: upload, access-token: '${{ secrets.CS_ACCESS_TOKEN }}'}\n",
);

/// A binding of the token under another name, as any `env` could carry it.
const BINDING: &str = "{T: '${{ secrets.CS_ACCESS_TOKEN }}'}";

/// Where, if anywhere, a generated case binds the token in an `env`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// No `env` names the token, which is the required shape.
    Nowhere,
    /// The workflow's `env`, which reaches every step of every job.
    Workflow,
    /// The job's `env`, which reaches every step of the job.
    Job,
    /// The upload step's `env`, which reaches every nested action step.
    Upload,
    /// The check step's `env`, which also disqualifies it as the check.
    Check,
    /// An unrelated step's `env`.
    Other,
}

/// Return one generated placement.
fn placement() -> impl Strategy<Value = Placement> {
    prop_oneof![
        Just(Placement::Nowhere),
        Just(Placement::Workflow),
        Just(Placement::Job),
        Just(Placement::Upload),
        Just(Placement::Check),
        Just(Placement::Other),
    ]
}

/// Return an unrelated step, binding the token when `is_holder`.
fn unrelated(index: usize, is_holder: bool) -> String {
    let env = if is_holder {
        format!("        env: {BINDING}\n")
    } else {
        String::new()
    };
    format!("      - run: make step-{index}\n{env}")
}

/// Return `step` with the token bound in its `env` when `is_holder`.
///
/// The binding goes after the step's first line, so it joins the step's
/// own mapping rather than starting a new one.
fn bound(step: &str, is_holder: bool) -> String {
    match step.split_once('\n') {
        Some((head, rest)) if is_holder => format!("{head}\n        env: {BINDING}\n{rest}"),
        _ => step.to_owned(),
    }
}

/// Return the publisher workflow text for one generated case.
fn publisher(others: usize, check_at: usize, upload_at: usize, at: Placement) -> String {
    let mut steps: Vec<String> = (0..others)
        .map(|index| unrelated(index, at == Placement::Other && index == 0))
        .collect();
    let upload = bound(UPLOAD, at == Placement::Upload);
    let check = bound(CHECK, at == Placement::Check);
    steps.insert(upload_at.min(steps.len()), upload);
    steps.insert(check_at.min(steps.len()), check);
    let workflow_env = if at == Placement::Workflow {
        format!("env: {BINDING}\n")
    } else {
        String::new()
    };
    let job_env = if at == Placement::Job {
        format!("    env: {BINDING}\n")
    } else {
        String::new()
    };
    format!(
        "on:\n  push:\n    branches: [main]\n{workflow_env}jobs:\n  upload:\n{job_env}    steps:\n{}",
        steps.concat()
    )
}

proptest! {
    #[test]
    fn only_a_checked_upload_with_no_token_in_env_passes(
        others in 0_usize..4,
        check_at in 0_usize..6,
        upload_at in 0_usize..6,
        at in placement(),
    ) {
        // An unrelated step is needed for the token to sit on one.
        prop_assume!(at != Placement::Other || others > 0);
        let text = publisher(others, check_at, upload_at, at);
        let workflows = vec![parse_workflow("main.yml", &text)
            .map_err(|err| TestCaseError::fail(err.to_string()))?];
        let upload_index = upload_at.min(others);
        let is_check_first = check_at.min(others + 1) <= upload_index;
        let is_checked = is_check_first && at != Placement::Check;
        prop_assert_eq!(guard_faults(&workflows).is_empty(), is_checked, "{}", text);
        prop_assert_eq!(
            token_faults(&workflows).is_empty(),
            at == Placement::Nowhere,
            "{}",
            text
        );
    }
}

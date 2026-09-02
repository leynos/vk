//! Guard VK's repository-owned Linux runner assignments.

use std::collections::BTreeMap;

const NAMESPACE_RUNNER: &str = "namespace-profile-default";

#[test]
fn repository_owned_linux_workflows_use_the_shared_namespace_profile() {
    for (workflow_name, workflow, expected_assignments) in [
        (
            "coverage",
            include_str!("../.github/workflows/coverage.yml"),
            2,
        ),
        (
            "main coverage",
            include_str!("../.github/workflows/coverage-main.yml"),
            1,
        ),
        (
            "delayed PR comment",
            include_str!("../.github/workflows/delayed-pr-comment.yml"),
            1,
        ),
        (
            "release",
            include_str!("../.github/workflows/release.yml"),
            2,
        ),
    ] {
        let runner_assignments: Vec<_> = workflow
            .lines()
            .filter_map(|line| line.trim().strip_prefix("runs-on:"))
            .map(str::trim)
            .collect();

        assert_eq!(
            runner_assignments.len(),
            expected_assignments,
            "{workflow_name} must assign every direct Linux job to {NAMESPACE_RUNNER}"
        );
        assert!(
            runner_assignments
                .iter()
                .all(|runner| *runner == NAMESPACE_RUNNER),
            "{workflow_name} must assign every direct Linux job to {NAMESPACE_RUNNER}"
        );
    }
}

#[test]
fn repository_owned_workflows_declare_exact_job_permissions() {
    assert_eq!(
        extract_job_permissions(include_str!("../.github/workflows/coverage.yml")),
        BTreeMap::from([
            (
                "build-test".to_owned(),
                BTreeMap::from([("contents".to_owned(), "read".to_owned())]),
            ),
            (
                "unstable-rest-resolve".to_owned(),
                BTreeMap::from([("contents".to_owned(), "read".to_owned())]),
            ),
        ])
    );
    assert_eq!(
        extract_job_permissions(include_str!("../.github/workflows/coverage-main.yml")),
        BTreeMap::from([(
            "coverage-upload".to_owned(),
            BTreeMap::from([("contents".to_owned(), "read".to_owned())]),
        )])
    );
    assert_eq!(
        extract_job_permissions(include_str!("../.github/workflows/delayed-pr-comment.yml")),
        BTreeMap::from([(
            "delay_and_comment".to_owned(),
            BTreeMap::from([("pull-requests".to_owned(), "write".to_owned())]),
        )])
    );
    assert_eq!(
        extract_job_permissions(include_str!("../.github/workflows/release.yml")),
        BTreeMap::from([
            (
                "build".to_owned(),
                BTreeMap::from([("contents".to_owned(), "read".to_owned())]),
            ),
            (
                "release".to_owned(),
                BTreeMap::from([("contents".to_owned(), "write".to_owned())]),
            ),
        ])
    );
}

#[test]
fn codescene_tokens_are_scoped_to_upload_steps() {
    assert_eq!(
        extract_token_exposures(include_str!("../.github/workflows/coverage.yml")),
        vec![(
            "build-test".to_owned(),
            "Check coverage against CodeScene gates".to_owned(),
        )]
    );
    assert_eq!(
        extract_token_exposures(include_str!("../.github/workflows/coverage-main.yml")),
        vec![(
            "coverage-upload".to_owned(),
            "Upload coverage data to CodeScene".to_owned(),
        )]
    );
    assert!(
        extract_token_exposures(include_str!("../.github/workflows/delayed-pr-comment.yml"))
            .is_empty()
    );
    assert!(extract_token_exposures(include_str!("../.github/workflows/release.yml")).is_empty());
}

#[test]
fn actionlint_allows_the_selected_namespace_runner_only() {
    assert_eq!(
        include_str!("../.github/actionlint.yaml").trim(),
        "self-hosted-runner:\n  labels:\n    - namespace-profile-default"
    );
}

/// Extract job-level permissions from a workflow's small, stable YAML shape.
fn extract_job_permissions(workflow: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut jobs = BTreeMap::new();
    let mut current_job = None;
    let mut job_permissions = BTreeMap::new();
    let mut reading_permissions = false;
    let mut in_jobs = false;

    for line in workflow.lines() {
        let indentation = line.len() - line.trim_start().len();
        let content = line.trim();

        if indentation == 0 && !content.is_empty() {
            in_jobs = content == "jobs:";
        } else if in_jobs && indentation == 2 && content.ends_with(':') {
            if let Some(job) = current_job.take() {
                jobs.insert(job, job_permissions);
            }
            current_job = Some(content.trim_end_matches(':').to_owned());
            job_permissions = BTreeMap::new();
            reading_permissions = false;
            continue;
        }

        if indentation == 4 && content == "permissions:" {
            reading_permissions = true;
            continue;
        }

        if reading_permissions && indentation == 6 {
            if let Some((name, value)) = content.split_once(':') {
                let value = value.split('#').next().map(str::trim).unwrap_or_default();
                job_permissions.insert(name.to_owned(), value.to_owned());
            }
        } else if reading_permissions && indentation <= 4 {
            reading_permissions = false;
        }
    }

    if let Some(job) = current_job {
        jobs.insert(job, job_permissions);
    }

    jobs
}

/// Extract each `CodeScene` token's owning job and step from workflow YAML.
fn extract_token_exposures(workflow: &str) -> Vec<(String, String)> {
    let mut exposures = Vec::new();
    let mut current_job = None;
    let mut current_step = None;

    for line in workflow.lines() {
        let indentation = line.len() - line.trim_start().len();
        let content = line.trim();

        if indentation == 2 && content.ends_with(':') {
            current_job = Some(content.trim_end_matches(':').to_owned());
            current_step = None;
        } else if indentation == 6 {
            if let Some(step_name) = content.strip_prefix("- name:") {
                current_step = Some(step_name.trim().to_owned());
            }
        } else if content.starts_with("CS_ACCESS_TOKEN:")
            && let Some(job) = current_job.clone()
        {
            let step = current_step
                .clone()
                .unwrap_or_else(|| "<job-level>".to_owned());
            exposures.push((job, step));
        }
    }

    exposures
}

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
    let mut parser = JobPermissionsParser::default();
    workflow_lines(workflow)
        .for_each(|(indentation, content)| parser.consume(indentation, content));
    parser.finish()
}

/// Extract each `CodeScene` token's owning job and step from workflow YAML.
fn extract_token_exposures(workflow: &str) -> Vec<(String, String)> {
    let mut parser = TokenExposureParser::default();
    workflow_lines(workflow)
        .for_each(|(indentation, content)| parser.consume(indentation, content));
    parser.finish()
}

/// Split workflow text into indentation and trimmed content for the parsers.
fn workflow_lines(workflow: &str) -> impl Iterator<Item = (usize, &str)> {
    workflow.lines().map(|line| {
        let indentation = line.len() - line.trim_start().len();
        (indentation, line.trim())
    })
}

/// Identify a non-empty top-level workflow key.
fn is_workflow_jobs_header(indentation: usize, content: &str) -> bool {
    indentation == 0 && !content.is_empty()
}

#[derive(Default)]
struct JobPermissionsParser {
    jobs: BTreeMap<String, BTreeMap<String, String>>,
    current_job: Option<String>,
    job_permissions: BTreeMap<String, String>,
    reading_permissions: bool,
    in_jobs: bool,
}

impl JobPermissionsParser {
    /// Consume one workflow line while retaining only job-level permissions.
    fn consume(&mut self, indentation: usize, content: &str) {
        if self.update_jobs_section(indentation, content) {
            return;
        }
        if self.start_permissions(indentation, content) {
            return;
        }
        self.read_permission(indentation, content);
    }

    /// Track entry into and between top-level workflow job definitions.
    fn update_jobs_section(&mut self, indentation: usize, content: &str) -> bool {
        if is_workflow_jobs_header(indentation, content) {
            self.in_jobs = content == "jobs:";
            return true;
        }
        if self.is_job_header(indentation, content) {
            self.store_current_job();
            self.current_job = Some(content.trim_end_matches(':').to_owned());
            self.job_permissions = BTreeMap::new();
            self.reading_permissions = false;
            return true;
        }
        false
    }

    /// Identify a job header while the parser is inside the jobs section.
    fn is_job_header(&self, indentation: usize, content: &str) -> bool {
        self.in_jobs && indentation == 2 && content.ends_with(':')
    }

    /// Start collecting a job's permission entries when its block begins.
    fn start_permissions(&mut self, indentation: usize, content: &str) -> bool {
        if indentation == 4 && content == "permissions:" {
            self.reading_permissions = true;
            return true;
        }
        false
    }

    /// Record a permission entry or stop collecting at the next job key.
    fn read_permission(&mut self, indentation: usize, content: &str) {
        if !self.reading_permissions {
            return;
        }
        if indentation == 6 {
            self.record_permission(content);
        } else if indentation <= 4 {
            self.reading_permissions = false;
        }
    }

    /// Store one permission entry after removing an inline comment.
    fn record_permission(&mut self, content: &str) {
        if let Some((name, value)) = content.split_once(':') {
            let value = value.split('#').next().map(str::trim).unwrap_or_default();
            self.job_permissions
                .insert(name.to_owned(), value.to_owned());
        }
    }

    /// Store the current job before moving to another job or finishing.
    fn store_current_job(&mut self) {
        if let Some(job) = self.current_job.take() {
            self.jobs
                .insert(job, std::mem::take(&mut self.job_permissions));
        }
    }

    /// Finish parsing and return all discovered job permission maps.
    fn finish(mut self) -> BTreeMap<String, BTreeMap<String, String>> {
        self.store_current_job();
        self.jobs
    }
}

#[derive(Default)]
struct TokenExposureParser {
    exposures: Vec<(String, String)>,
    current_job: Option<String>,
    current_step: Option<String>,
}

impl TokenExposureParser {
    /// Consume one workflow line while recording `CodeScene` token locations.
    fn consume(&mut self, indentation: usize, content: &str) {
        if self.update_job(indentation, content) {
            return;
        }
        if self.update_step(indentation, content) {
            return;
        }
        if content.starts_with("CS_ACCESS_TOKEN:") {
            self.record_token();
        }
    }

    /// Track the job that owns the current workflow line.
    fn update_job(&mut self, indentation: usize, content: &str) -> bool {
        if indentation == 2 && content.ends_with(':') {
            self.current_job = Some(content.trim_end_matches(':').to_owned());
            self.current_step = None;
            return true;
        }
        false
    }

    /// Track named steps so token exposure can be checked at step scope.
    fn update_step(&mut self, indentation: usize, content: &str) -> bool {
        if indentation == 6 {
            if let Some(step_name) = content.strip_prefix("- name:") {
                self.current_step = Some(step_name.trim().to_owned());
            }
            return true;
        }
        false
    }

    /// Record the current token location, including job-level exposure.
    fn record_token(&mut self) {
        if let Some(job) = self.current_job.clone() {
            let step = self
                .current_step
                .clone()
                .unwrap_or_else(|| "<job-level>".to_owned());
            self.exposures.push((job, step));
        }
    }

    /// Finish parsing and return all discovered token locations.
    fn finish(self) -> Vec<(String, String)> {
        self.exposures
    }
}

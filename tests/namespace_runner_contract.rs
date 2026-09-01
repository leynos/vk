//! Guard VK's repository-owned Linux runner assignments.

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

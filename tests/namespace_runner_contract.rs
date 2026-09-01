//! Guard VK's repository-owned Linux runner assignments.

const NAMESPACE_RUNNER: &str = "runs-on: namespace-profile-default";

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
        assert_eq!(
            workflow.matches(NAMESPACE_RUNNER).count(),
            expected_assignments,
            "{workflow_name} must assign every direct Linux job to {NAMESPACE_RUNNER}"
        );
    }
}

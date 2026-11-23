//! Tests for `ref_parser` module.

use super::*;
use crate::test_utils::GitRepoFixture;
use rstest::{fixture, rstest};

/// rstest fixture for a repository on a feature branch.
#[fixture]
fn feature_branch_repo() -> GitRepoFixture {
    GitRepoFixture::on_branch("feature-branch").expect("init feature-branch fixture")
}

/// rstest fixture for a repository with detached HEAD.
#[fixture]
fn detached_head_repo() -> GitRepoFixture {
    GitRepoFixture::detached().expect("init detached HEAD fixture")
}

#[test]
fn parse_url() {
    let (repo, number) =
        parse_pr_reference("https://github.com/owner/repo/pull/42", None).expect("valid reference");
    assert_eq!(repo.owner, "owner");
    assert_eq!(repo.name, "repo");
    assert_eq!(number, 42);
}

#[test]
fn parse_url_git_suffix() {
    let (repo, number) = parse_pr_reference("https://github.com/owner/repo.git/pull/7", None)
        .expect("valid reference");
    assert_eq!(repo.owner, "owner");
    assert_eq!(repo.name, "repo");
    assert_eq!(number, 7);
}

#[test]
fn parse_url_dotted_repo_name() {
    let (repo, number) = parse_pr_reference("https://github.com/owner/my.repo.git/pull/5", None)
        .expect("valid reference");
    assert_eq!(repo.owner, "owner");
    assert_eq!(repo.name, "my.repo");
    assert_eq!(number, 5);
}

#[test]
fn parse_url_plural_segment() {
    let (repo, number) = parse_pr_reference("https://github.com/owner/repo/pulls/13", None)
        .expect("valid reference");
    assert_eq!(repo.owner, "owner");
    assert_eq!(repo.name, "repo");
    assert_eq!(number, 13);
}

#[test]
fn repo_from_fetch_head_git_suffix() {
    let fixture = GitRepoFixture::on_branch("main")
        .and_then(|f| {
            f.with_fetch_head(
                "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/foo/bar.git",
            )
        })
        .expect("build FETCH_HEAD fixture");

    let repo = repo_from_fetch_head_impl(Some(fixture.path())).expect("repo from fetch head");
    assert_eq!(repo.owner, "foo");
    assert_eq!(repo.name, "bar");
}

#[test]
fn parse_repo_str_git_suffix() {
    let repo = parse_repo_str("a/b.git").expect("parse repo");
    assert_eq!(repo.owner, "a");
    assert_eq!(repo.name, "b");
}

#[test]
fn parse_issue_url() {
    let (repo, number) =
        parse_issue_reference("https://github.com/owner/repo/issues/3", None).expect("valid ref");
    assert_eq!(repo.owner, "owner");
    assert_eq!(repo.name, "repo");
    assert_eq!(number, 3);
}

#[test]
fn parse_issue_url_plural() {
    let (repo, number) =
        parse_issue_reference("https://github.com/owner/repo/issues/31", None).expect("valid ref");
    assert_eq!(repo.owner, "owner");
    assert_eq!(repo.name, "repo");
    assert_eq!(number, 31);
}

#[test]
fn parse_issue_url_git_suffix() {
    let (repo, number) = parse_issue_reference("https://github.com/owner/repo.git/issues/9", None)
        .expect("valid ref");
    assert_eq!(repo.owner, "owner");
    assert_eq!(repo.name, "repo");
    assert_eq!(number, 9);
}

#[test]
fn parse_issue_url_singular() {
    let (repo, number) =
        parse_issue_reference("https://github.com/owner/repo/issue/11", None).expect("valid ref");
    assert_eq!(repo.owner, "owner");
    assert_eq!(repo.name, "repo");
    assert_eq!(number, 11);
}

#[test]
fn parse_pr_number_with_repo() {
    let (repo, number) = parse_pr_reference("5", Some("foo/bar")).expect("valid ref");
    assert_eq!(repo.owner, "foo");
    assert_eq!(repo.name, "bar");
    assert_eq!(number, 5);
}

#[test]
fn parse_issue_number_with_repo() {
    let (repo, number) = parse_issue_reference("8", Some("baz/qux")).expect("valid ref");
    assert_eq!(repo.owner, "baz");
    assert_eq!(repo.name, "qux");
    assert_eq!(number, 8);
}

#[test]
fn parse_pr_thread_reference_with_comment() {
    let (repo, number, comment) =
        parse_pr_thread_reference("https://github.com/owner/repo/pull/1#discussion_r99", None)
            .expect("parse");
    assert_eq!(repo.owner, "owner");
    assert_eq!(repo.name, "repo");
    assert_eq!(number, 1);
    assert_eq!(comment, Some(99));
}

#[rstest]
#[case("https://github.com/o/r/pull/1#discussion_r")]
#[case("https://github.com/o/r/pull/1#discussion_rabc")]
fn parse_pr_thread_reference_rejects_bad_fragment(#[case] input: &str) {
    let err = parse_pr_thread_reference(input, None).expect_err("invalid ref");
    assert!(matches!(err, VkError::InvalidRef));
}

#[rstest]
fn current_branch_parses_symbolic_ref(feature_branch_repo: GitRepoFixture) {
    let branch = current_branch_impl(Some(feature_branch_repo.path())).expect("branch from HEAD");
    assert_eq!(branch, "feature-branch");
}

#[rstest]
fn current_branch_returns_none_for_detached_head(detached_head_repo: GitRepoFixture) {
    assert!(current_branch_impl(Some(detached_head_repo.path())).is_none());
}

#[rstest]
#[case("#discussion_r123", true)]
#[case("#discussion_r1", true)]
#[case("42#discussion_r123", false)]
#[case("https://github.com/o/r/pull/1#discussion_r123", false)]
#[case("", false)]
#[case("#discussion_", false)]
fn is_fragment_only_detects_bare_fragments(#[case] input: &str, #[case] expected: bool) {
    assert_eq!(is_fragment_only(input), expected);
}

#[test]
fn parse_fragment_only_extracts_comment_id() {
    assert_eq!(parse_fragment_only("#discussion_r123").expect("parse"), 123);
    assert_eq!(parse_fragment_only("#discussion_r1").expect("parse"), 1);
}

#[rstest]
#[case("#discussion_r")]
#[case("#discussion_rabc")]
#[case("42#discussion_r123")]
#[case("")]
fn parse_fragment_only_rejects_invalid_input(#[case] input: &str) {
    assert!(parse_fragment_only(input).is_err());
}

#[test]
fn repo_from_origin_extracts_owner_and_name() {
    let fixture = GitRepoFixture::on_branch("main")
        .and_then(|f| f.with_origin("https://github.com/fork-owner/my-repo.git"))
        .expect("build origin fixture");

    let repo = repo_from_origin_impl(Some(fixture.path())).expect("repo from origin");
    assert_eq!(repo.owner, "fork-owner");
    assert_eq!(repo.name, "my-repo");
}

#[test]
fn repo_from_origin_returns_none_without_remote() {
    let fixture = GitRepoFixture::on_branch("main").expect("init main fixture");

    assert!(repo_from_origin_impl(Some(fixture.path())).is_none());
}

/// Resolve a bare number as `resource_type` with git run in the fixture's directory.
fn parse_bare_in(
    fixture: &GitRepoFixture,
    input: &str,
    resource_type: ResourceType,
) -> Result<(RepoInfo, u64), VkError> {
    parse_reference(
        input,
        DefaultRepo::from(None),
        resource_type,
        Some(fixture.path()),
    )
}

/// A fresh worktree where `git fetch` has not yet been run: no `FETCH_HEAD`,
/// but `origin` is configured.
fn origin_only_fixture() -> GitRepoFixture {
    GitRepoFixture::on_branch("feature-branch")
        .and_then(|f| f.with_origin("https://github.com/leynos/chutoro.git"))
        .expect("build origin-only fixture")
}

/// A fork workflow: `origin` is the fork and `FETCH_HEAD` names the upstream.
fn fork_and_upstream_fixture() -> GitRepoFixture {
    GitRepoFixture::on_branch("feature-branch")
        .and_then(|f| f.with_origin("https://github.com/fork/repo.git"))
        .and_then(|f| {
            f.with_fetch_head(
                "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/upstream/repo.git",
            )
        })
        .expect("build fork-and-upstream fixture")
}

// Issue-number resolution shares the bare-number code path with PRs via
// `parse_reference`, so both resource types run the same cases: a future
// divergence between the two entry points cannot regress either unnoticed.

#[rstest]
#[case::pull_request(ResourceType::PullRequest)]
#[case::issue(ResourceType::Issues)]
fn bare_number_falls_back_to_origin_without_fetch_head(#[case] resource_type: ResourceType) {
    let fixture = origin_only_fixture();

    let (repo, number) = parse_bare_in(&fixture, "17", resource_type)
        .expect("origin should resolve repo for a bare number");
    assert_eq!(repo.owner, "leynos");
    assert_eq!(repo.name, "chutoro");
    assert_eq!(number, 17);
}

#[rstest]
#[case::pull_request(ResourceType::PullRequest)]
#[case::issue(ResourceType::Issues)]
fn bare_number_prefers_fetch_head_over_origin(#[case] resource_type: ResourceType) {
    let fixture = fork_and_upstream_fixture();

    let (repo, _) = parse_bare_in(&fixture, "3", resource_type).expect("FETCH_HEAD should win");
    assert_eq!(repo.owner, "upstream");
    assert_eq!(repo.name, "repo");
}

/// Environment variable that makes the re-executed test binary run the
/// public-entry-point checks instead of spawning another child.
const PUBLIC_ENTRY_POINTS_CHILD: &str = "VK_REF_PARSER_PUBLIC_ENTRY_POINTS_CHILD";

/// The public entry points use the process working directory. Run them in a
/// child process whose directory is the fixture, so this test never changes
/// the parent's directory.
#[rstest]
#[case::origin_fallback(origin_only_fixture(), "leynos")]
#[case::fetch_head_precedence(fork_and_upstream_fixture(), "upstream")]
fn public_entry_points_use_the_process_working_directory(
    #[case] fixture: GitRepoFixture,
    #[case] expected_owner: &str,
) {
    let test_name = format!(
        "{}::public_entry_points_child",
        module_path!()
            .split_once("::")
            .map_or(module_path!(), |(_, rest)| rest)
    );
    let output = std::process::Command::new(std::env::current_exe().expect("test binary path"))
        .args(["--exact", &test_name, "--nocapture", "--test-threads=1"])
        .env(PUBLIC_ENTRY_POINTS_CHILD, expected_owner)
        .current_dir(fixture.path())
        .output()
        .expect("spawn the child test process");
    assert!(
        output.status.success(),
        "child failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("public entry points resolved"),
        "the child did not reach its assertions (test name filter?): {stdout}",
    );
}

/// Child half of the test above: runs only when the parent set the variable.
#[test]
fn public_entry_points_child() {
    let Some(expected_owner) = std::env::var_os(PUBLIC_ENTRY_POINTS_CHILD) else {
        return;
    };
    let expected_owner = expected_owner.to_string_lossy();
    let (pr_repo, pr_number) = parse_pr_reference("5", None).expect("PR number should resolve");
    let (issue_repo, issue_number) =
        parse_issue_reference("5", None).expect("issue number should resolve");
    assert_eq!(pr_repo.owner, expected_owner);
    assert_eq!(issue_repo.owner, expected_owner);
    assert_eq!((pr_number, issue_number), (5, 5));
    println!("public entry points resolved");
}

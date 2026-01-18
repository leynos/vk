//! Tests for `ref_parser` module.

use super::*;
use rstest::{fixture, rstest};
use std::fs;
use tempfile::TempDir;

/// A temporary Git repository directory for testing.
///
/// This struct manages a temporary directory containing an initialized Git
/// repository. Tests should use the `path()` method to get the directory
/// path and pass it to commands via `current_dir()`.
struct GitRepoFixture {
    dir: TempDir,
}

impl GitRepoFixture {
    /// Create a fixture with a symbolic ref to a branch.
    ///
    /// Initializes a git repository and uses `git symbolic-ref` to set HEAD
    /// to point to the specified branch without requiring a commit.
    fn on_branch(branch: &str) -> Self {
        use std::process::Command;

        let dir = TempDir::new().expect("tempdir");

        // Initialize a real git repository
        // Use -c init.defaultBranch=main for compatibility with Git < 2.28
        let status = Command::new("git")
            .args(["-c", "init.defaultBranch=main", "init"])
            .current_dir(dir.path())
            .output()
            .expect("git init");
        assert!(status.status.success(), "git init failed");

        // Use git symbolic-ref to set HEAD to desired branch
        let status = Command::new("git")
            .args(["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")])
            .current_dir(dir.path())
            .output()
            .expect("git symbolic-ref");
        assert!(status.status.success(), "git symbolic-ref failed");

        Self { dir }
    }

    /// Create a fixture with a detached HEAD.
    ///
    /// Initializes a git repository, creates an initial commit, then
    /// detaches HEAD to that commit.
    fn detached() -> Self {
        use std::process::Command;

        let dir = TempDir::new().expect("tempdir");

        // Initialize a real git repository
        let status = Command::new("git")
            .args(["-c", "init.defaultBranch=main", "init"])
            .current_dir(dir.path())
            .output()
            .expect("git init");
        assert!(status.status.success(), "git init failed");

        // Configure user for commit
        let status = Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .expect("git config email");
        assert!(status.status.success(), "git config email failed");
        let status = Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .expect("git config name");
        assert!(status.status.success(), "git config name failed");

        // Create an empty commit so we have something to detach to
        let status = Command::new("git")
            .args(["commit", "--allow-empty", "-m", "initial"])
            .current_dir(dir.path())
            .output()
            .expect("git commit");
        assert!(status.status.success(), "git commit failed");

        // Detach HEAD
        let status = Command::new("git")
            .args(["checkout", "--detach"])
            .current_dir(dir.path())
            .output()
            .expect("git checkout --detach");
        assert!(status.status.success(), "git checkout --detach failed");

        Self { dir }
    }

    /// Add an origin remote to the repository.
    ///
    /// Configures the origin remote to point to the given URL.
    fn with_origin(self, url: &str) -> Self {
        use std::process::Command;

        let status = Command::new("git")
            .args(["remote", "add", "origin", url])
            .current_dir(self.dir.path())
            .output()
            .expect("git remote add");
        assert!(status.status.success(), "git remote add failed");

        self
    }

    /// Write `FETCH_HEAD` content to the repository.
    fn with_fetch_head(self, content: &str) -> Self {
        let git_dir = self.dir.path().join(".git");
        fs::write(git_dir.join("FETCH_HEAD"), content).expect("write FETCH_HEAD");
        self
    }

    /// Get the path to the temporary git repository.
    fn path(&self) -> &std::path::Path {
        self.dir.path()
    }
}

/// rstest fixture for a repository on a feature branch.
#[fixture]
fn feature_branch_repo() -> GitRepoFixture {
    GitRepoFixture::on_branch("feature-branch")
}

/// rstest fixture for a repository with detached HEAD.
#[fixture]
fn detached_head_repo() -> GitRepoFixture {
    GitRepoFixture::detached()
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
    let fixture = GitRepoFixture::on_branch("main").with_fetch_head(
        "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/foo/bar.git",
    );

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
    let fixture =
        GitRepoFixture::on_branch("main").with_origin("https://github.com/fork-owner/my-repo.git");

    let repo = repo_from_origin_impl(Some(fixture.path())).expect("repo from origin");
    assert_eq!(repo.owner, "fork-owner");
    assert_eq!(repo.name, "my-repo");
}

#[test]
fn repo_from_origin_returns_none_without_remote() {
    let fixture = GitRepoFixture::on_branch("main");

    assert!(repo_from_origin_impl(Some(fixture.path())).is_none());
}

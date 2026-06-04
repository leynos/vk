//! Tests for command helper utilities.

use super::handle_banner;
use super::locale_is_utf8;
use crate::test_utils::{apply_optional_env, restore_optional_env};
use rstest::{fixture, rstest};
use serial_test::serial;
use vk::environment;

struct LocaleEnvGuard {
    lc_all: Option<String>,
    lc_ctype: Option<String>,
    lang: Option<String>,
}

impl Drop for LocaleEnvGuard {
    fn drop(&mut self) {
        restore_optional_env("LC_ALL", self.lc_all.take());
        restore_optional_env("LC_CTYPE", self.lc_ctype.take());
        restore_optional_env("LANG", self.lang.take());
    }
}

#[fixture]
fn locale_env() -> LocaleEnvGuard {
    LocaleEnvGuard {
        lc_all: environment::var("LC_ALL").ok(),
        lc_ctype: environment::var("LC_CTYPE").ok(),
        lang: environment::var("LANG").ok(),
    }
}

fn apply_locale(lc_all: Option<&str>, lc_ctype: Option<&str>, lang: Option<&str>) {
    apply_optional_env("LC_ALL", lc_all);
    apply_optional_env("LC_CTYPE", lc_ctype);
    apply_optional_env("LANG", lang);
}

#[rstest]
#[case(Some("en_GB.UTF-8"), None, None, true)]
#[case(Some("en_GB.UTF8"), None, None, true)]
#[case(Some("en_GB.utf8"), None, None, true)]
#[case(Some("en_GB.UTF80"), None, None, false)]
#[case(None, Some("en_GB.UTF-8"), None, true)]
#[case(None, Some("C"), None, false)]
#[case(None, None, Some("en_GB.UTF-8"), true)]
#[case(None, None, Some("C"), false)]
#[case(None, None, None, false)]
#[serial]
fn detect_utf8_locale_cases(
    #[case] lc_all: Option<&str>,
    #[case] lc_ctype: Option<&str>,
    #[case] lang: Option<&str>,
    #[case] expected: bool,
    locale_env: LocaleEnvGuard,
) {
    let _ = locale_env;
    apply_locale(lc_all, lc_ctype, lang);
    assert_eq!(locale_is_utf8(), expected);
}

#[test]
fn handle_banner_returns_true_on_broken_pipe() {
    let broken_pipe =
        || -> std::io::Result<()> { Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe)) };
    assert!(handle_banner(broken_pipe, "start"));
}

#[test]
fn handle_banner_logs_and_returns_false_on_other_errors() {
    let other_err = || -> std::io::Result<()> { Err(std::io::Error::other("boom")) };
    assert!(!handle_banner(other_err, "end"));
}

mod resolve_branch_and_repo_tests {
    use super::super::resolve_branch_and_repo;
    use rstest::{fixture, rstest};
    use serial_test::serial;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::{TempDir, tempdir};

    /// A temporary Git repository directory for testing `resolve_branch_and_repo`.
    struct GitRepoFixture {
        dir: TempDir,
        original_cwd: PathBuf,
    }

    impl GitRepoFixture {
        /// Create a new fixture with a git repository on the specified branch.
        ///
        /// The repository has no `FETCH_HEAD`, no origin remote, and no
        /// commits; HEAD is a symbolic ref to the requested branch.
        fn on_branch(branch: &str) -> Self {
            let dir = tempdir().expect("tempdir");
            let original_cwd = std::env::current_dir().expect("cwd");

            // Use -c init.defaultBranch=main for compatibility with Git < 2.28
            let status = Command::new("git")
                .args(["-c", "init.defaultBranch=main", "init"])
                .current_dir(dir.path())
                .output()
                .expect("git init");
            assert!(status.status.success(), "git init failed");

            let status = Command::new("git")
                .args(["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")])
                .current_dir(dir.path())
                .output()
                .expect("git symbolic-ref");
            assert!(status.status.success(), "git symbolic-ref failed");

            std::env::set_current_dir(dir.path()).expect("chdir temp");
            Self { dir, original_cwd }
        }

        /// Write `FETCH_HEAD` content into the repository.
        fn with_fetch_head(self, content: &str) -> Self {
            let git_dir = self.dir.path().join(".git");
            fs::write(git_dir.join("FETCH_HEAD"), content).expect("write FETCH_HEAD");
            self
        }

        /// Add an `origin` remote pointing at `url`.
        fn with_origin(self, url: &str) -> Self {
            let status = Command::new("git")
                .args(["remote", "add", "origin", url])
                .current_dir(self.dir.path())
                .output()
                .expect("git remote add");
            assert!(status.status.success(), "git remote add failed");
            self
        }

        /// Create a new fixture with a git repository on the specified branch
        /// and with `FETCH_HEAD` containing the specified repo URL.
        fn with_branch_and_fetch_head(branch: &str, fetch_head_content: &str) -> Self {
            Self::on_branch(branch).with_fetch_head(fetch_head_content)
        }
    }

    impl Drop for GitRepoFixture {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original_cwd);
        }
    }

    /// rstest fixture for a repository on feature-branch with `FETCH_HEAD`.
    #[fixture]
    fn git_repo_on_feature_branch() -> GitRepoFixture {
        GitRepoFixture::with_branch_and_fetch_head(
            "feature-branch",
            "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/fallback/repo.git",
        )
    }

    #[rstest]
    #[serial]
    fn returns_repo_from_default_repo_when_provided(git_repo_on_feature_branch: GitRepoFixture) {
        let _fixture = git_repo_on_feature_branch;
        let result = resolve_branch_and_repo(Some("owner/repo"));
        let ctx = result.expect("should resolve successfully");
        assert_eq!(ctx.repo.owner, "owner", "should use provided repo owner");
        assert_eq!(ctx.repo.name, "repo", "should use provided repo name");
        assert_eq!(
            ctx.branch, "feature-branch",
            "should detect branch from git"
        );
        // No origin remote configured in test fixture, so head_owner should be None
        assert!(ctx.head_owner.is_none(), "no origin remote configured");
    }

    #[rstest]
    #[serial]
    fn falls_back_to_fetch_head_when_no_default_repo(git_repo_on_feature_branch: GitRepoFixture) {
        let _fixture = git_repo_on_feature_branch;
        let result = resolve_branch_and_repo(None);
        let ctx = result.expect("should resolve successfully");
        assert_eq!(
            ctx.repo.owner, "fallback",
            "should use FETCH_HEAD repo owner"
        );
        assert_eq!(ctx.repo.name, "repo", "should use FETCH_HEAD repo name");
        assert_eq!(
            ctx.branch, "feature-branch",
            "should detect branch from git"
        );
    }

    /// Fixture for a repo with origin remote pointing to a fork.
    #[fixture]
    fn git_repo_with_fork_origin() -> GitRepoFixture {
        let fixture = GitRepoFixture::with_branch_and_fetch_head(
            "feature-branch",
            "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/upstream/repo.git",
        );
        // Add origin remote pointing to the fork
        let status = Command::new("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/fork-owner/repo.git",
            ])
            .output()
            .expect("git remote add");
        assert!(status.status.success(), "git remote add failed");
        fixture
    }

    #[rstest]
    #[serial]
    fn extracts_head_owner_from_origin_remote(git_repo_with_fork_origin: GitRepoFixture) {
        let _fixture = git_repo_with_fork_origin;
        let result = resolve_branch_and_repo(Some("upstream/repo"));
        let ctx = result.expect("should resolve successfully");
        assert_eq!(ctx.repo.owner, "upstream", "target repo from --repo flag");
        assert_eq!(ctx.repo.name, "repo", "target repo name");
        assert_eq!(ctx.branch, "feature-branch", "branch from git");
        assert_eq!(
            ctx.head_owner.as_deref(),
            Some("fork-owner"),
            "head owner from origin remote"
        );
    }

    #[rstest]
    #[serial]
    fn prefers_fetch_head_over_origin_when_both_present(git_repo_with_fork_origin: GitRepoFixture) {
        let _fixture = git_repo_with_fork_origin;
        // FETCH_HEAD identifies the upstream in a fork workflow, so it must win
        // over the user's `origin` remote.
        let ctx = resolve_branch_and_repo(None).expect("should resolve successfully");
        assert_eq!(ctx.repo.owner, "upstream", "FETCH_HEAD takes precedence");
        assert_eq!(ctx.repo.name, "repo");
        assert_eq!(
            ctx.head_owner.as_deref(),
            Some("fork-owner"),
            "head owner still comes from origin"
        );
    }

    /// Fixture for a fresh worktree: branch and origin set, but no `FETCH_HEAD`
    /// yet. This mirrors the state of a `git worktree add` target where no
    /// `git fetch` has been run from inside the worktree.
    #[fixture]
    fn git_repo_with_origin_only() -> GitRepoFixture {
        GitRepoFixture::on_branch("feature-branch")
            .with_origin("https://github.com/leynos/chutoro.git")
    }

    #[rstest]
    #[serial]
    fn falls_back_to_origin_when_fetch_head_missing(git_repo_with_origin_only: GitRepoFixture) {
        let _fixture = git_repo_with_origin_only;
        let ctx =
            resolve_branch_and_repo(None).expect("origin must be used when FETCH_HEAD is absent");
        assert_eq!(ctx.repo.owner, "leynos", "origin owner");
        assert_eq!(ctx.repo.name, "chutoro", "origin repo name");
        assert_eq!(ctx.branch, "feature-branch");
        assert_eq!(
            ctx.head_owner.as_deref(),
            Some("leynos"),
            "head owner still derived from origin"
        );
    }
}

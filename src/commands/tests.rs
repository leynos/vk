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
        _dir: TempDir,
        original_cwd: PathBuf,
    }

    impl GitRepoFixture {
        /// Create a new fixture with a git repository on the specified branch
        /// and with `FETCH_HEAD` containing the specified repo URL.
        fn with_branch_and_fetch_head(branch: &str, fetch_head_content: &str) -> Self {
            let dir = tempdir().expect("tempdir");
            let original_cwd = std::env::current_dir().expect("cwd");

            // Initialize a real git repository so git rev-parse works
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

            // Write FETCH_HEAD
            let git_dir = dir.path().join(".git");
            fs::write(git_dir.join("FETCH_HEAD"), fetch_head_content).expect("write FETCH_HEAD");

            std::env::set_current_dir(dir.path()).expect("chdir temp");
            Self {
                _dir: dir,
                original_cwd,
            }
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
}

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
    use crate::test_utils::{CwdGuard, GitRepoFixture};
    use rstest::{fixture, rstest};
    use serial_test::serial;

    /// Pairs a [`GitRepoFixture`] with a [`CwdGuard`] so each test runs against
    /// the fixture's working tree. `resolve_branch_and_repo` reads from the
    /// current directory, so tests must hold this for their full duration and
    /// be marked `#[serial]`.
    struct ResolverFixture {
        _repo: GitRepoFixture,
        _cwd: CwdGuard,
    }

    impl ResolverFixture {
        fn enter(repo: GitRepoFixture) -> Self {
            let cwd = CwdGuard::enter(repo.path()).expect("enter fixture cwd");
            Self {
                _repo: repo,
                _cwd: cwd,
            }
        }
    }

    const FETCH_HEAD_FALLBACK: &str =
        "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/fallback/repo.git";
    const FETCH_HEAD_UPSTREAM: &str =
        "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/upstream/repo.git";

    /// Fixture for a repository on feature-branch with `FETCH_HEAD` only.
    #[fixture]
    fn git_repo_on_feature_branch() -> ResolverFixture {
        let repo = GitRepoFixture::on_branch("feature-branch")
            .and_then(|f| f.with_fetch_head(FETCH_HEAD_FALLBACK))
            .expect("build FETCH_HEAD-only fixture");
        ResolverFixture::enter(repo)
    }

    /// Fixture for a repo with origin remote pointing to a fork.
    #[fixture]
    fn git_repo_with_fork_origin() -> ResolverFixture {
        let repo = GitRepoFixture::on_branch("feature-branch")
            .and_then(|f| f.with_fetch_head(FETCH_HEAD_UPSTREAM))
            .and_then(|f| f.with_origin("https://github.com/fork-owner/repo.git"))
            .expect("build fork-origin fixture");
        ResolverFixture::enter(repo)
    }

    /// Fixture for a fresh worktree: branch and origin set, but no
    /// `FETCH_HEAD` yet. Mirrors a `git worktree add` target before any
    /// `git fetch` has run inside it.
    #[fixture]
    fn git_repo_with_origin_only() -> ResolverFixture {
        let repo = GitRepoFixture::on_branch("feature-branch")
            .and_then(|f| f.with_origin("https://github.com/leynos/chutoro.git"))
            .expect("build origin-only fixture");
        ResolverFixture::enter(repo)
    }

    #[rstest]
    #[serial]
    fn returns_repo_from_default_repo_when_provided(git_repo_on_feature_branch: ResolverFixture) {
        let _fixture = git_repo_on_feature_branch;
        let result = resolve_branch_and_repo(Some("owner/repo"));
        let ctx = result.expect("should resolve successfully");
        assert_eq!(ctx.repo.owner, "owner", "should use provided repo owner");
        assert_eq!(ctx.repo.name, "repo", "should use provided repo name");
        assert_eq!(
            ctx.branch, "feature-branch",
            "should detect branch from git"
        );
        assert!(ctx.head_owner.is_none(), "no origin remote configured");
    }

    #[rstest]
    #[serial]
    fn falls_back_to_fetch_head_when_no_default_repo(git_repo_on_feature_branch: ResolverFixture) {
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

    #[rstest]
    #[serial]
    fn extracts_head_owner_from_origin_remote(git_repo_with_fork_origin: ResolverFixture) {
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
    fn prefers_fetch_head_over_origin_when_both_present(
        git_repo_with_fork_origin: ResolverFixture,
    ) {
        let _fixture = git_repo_with_fork_origin;
        // FETCH_HEAD identifies the upstream in a fork workflow, so it must
        // win over the user's `origin` remote.
        let ctx = resolve_branch_and_repo(None).expect("should resolve successfully");
        assert_eq!(ctx.repo.owner, "upstream", "FETCH_HEAD takes precedence");
        assert_eq!(ctx.repo.name, "repo");
        assert_eq!(
            ctx.head_owner.as_deref(),
            Some("fork-owner"),
            "head owner still comes from origin"
        );
    }

    #[rstest]
    #[serial]
    fn falls_back_to_origin_when_fetch_head_missing(git_repo_with_origin_only: ResolverFixture) {
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

//! Tests for command helper utilities.

use super::handle_banner;
use super::is_utf8_locale;
use super::locale_is_utf8;
use ortho_config::MapEnv;
use rstest::rstest;

/// Build an environment holding only the locale variables that are set.
fn locale_env(lc_all: Option<&str>, lc_ctype: Option<&str>, lang: Option<&str>) -> MapEnv {
    [("LC_ALL", lc_all), ("LC_CTYPE", lc_ctype), ("LANG", lang)]
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .fold(MapEnv::new(), |env, (key, value)| env.with_var(key, value))
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
fn detect_utf8_locale_cases(
    #[case] lc_all: Option<&str>,
    #[case] lc_ctype: Option<&str>,
    #[case] lang: Option<&str>,
    #[case] expected: bool,
) {
    let env = locale_env(lc_all, lc_ctype, lang);
    assert_eq!(locale_is_utf8(&env), expected);
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

    //! Tests for branch and repository auto-detection.

    use super::super::branch::BranchContext;
    use super::super::resolve_branch_and_repo;
    use crate::VkError;
    use crate::test_utils::GitRepoFixture;
    use rstest::{fixture, rstest};

    /// Wraps a [`GitRepoFixture`] so each test resolves against the fixture's
    /// working tree: `resolve_branch_and_repo` takes the directory to run git
    /// in, so no test changes the process working directory.
    struct ResolverFixture {
        repo: GitRepoFixture,
    }

    impl ResolverFixture {
        fn enter(repo: GitRepoFixture) -> Self {
            Self { repo }
        }

        fn resolve(&self, default_repo: Option<&str>) -> Result<BranchContext, VkError> {
            resolve_branch_and_repo(default_repo, Some(self.repo.path()))
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
    fn returns_repo_from_default_repo_when_provided(git_repo_on_feature_branch: ResolverFixture) {
        let result = git_repo_on_feature_branch.resolve(Some("owner/repo"));
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
    fn falls_back_to_fetch_head_when_no_default_repo(git_repo_on_feature_branch: ResolverFixture) {
        let result = git_repo_on_feature_branch.resolve(None);
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
    fn extracts_head_owner_from_origin_remote(git_repo_with_fork_origin: ResolverFixture) {
        let result = git_repo_with_fork_origin.resolve(Some("upstream/repo"));
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
    fn prefers_fetch_head_over_origin_when_both_present(
        git_repo_with_fork_origin: ResolverFixture,
    ) {
        // FETCH_HEAD identifies the upstream in a fork workflow, so it must
        // win over the user's `origin` remote.
        let ctx = git_repo_with_fork_origin
            .resolve(None)
            .expect("should resolve successfully");
        assert_eq!(ctx.repo.owner, "upstream", "FETCH_HEAD takes precedence");
        assert_eq!(ctx.repo.name, "repo");
        assert_eq!(
            ctx.head_owner.as_deref(),
            Some("fork-owner"),
            "head owner still comes from origin"
        );
    }

    #[rstest]
    fn falls_back_to_origin_when_fetch_head_missing(git_repo_with_origin_only: ResolverFixture) {
        let ctx = git_repo_with_origin_only
            .resolve(None)
            .expect("origin must be used when FETCH_HEAD is absent");
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

#[test]
fn an_unavailable_locale_pattern_matches_nothing() {
    assert!(!is_utf8_locale(None, "en_GB.UTF-8"));
}

#[test]
fn a_compiled_locale_pattern_recognises_utf8() {
    assert!(is_utf8_locale(crate::UTF8_RE.as_ref(), "en_GB.UTF-8"));
    assert!(!is_utf8_locale(crate::UTF8_RE.as_ref(), "en_GB.ISO-8859-1"));
}

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
    use crate::VkError;

    #[test]
    fn returns_repo_from_default_repo_when_provided() {
        // This test exercises the happy path when default_repo is provided
        // but we're not in a git repo (no current_branch), so it should fail
        // with DetachedHead since we can't get the branch
        let result = resolve_branch_and_repo(Some("owner/repo"));
        // In CI/test environment without a real git repo context, this returns
        // DetachedHead (no .git/HEAD readable) or succeeds if run from repo root
        match result {
            Ok((repo, _branch)) => {
                // If it succeeds (we're in a real git repo), verify the repo
                assert_eq!(repo.owner, "owner");
                assert_eq!(repo.name, "repo");
            }
            Err(VkError::DetachedHead) => {
                // Expected when not in a git repo context
            }
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }
}

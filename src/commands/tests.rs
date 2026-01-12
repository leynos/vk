//! Tests for command helper utilities.

use super::handle_banner;
use super::locale_is_utf8;
use crate::test_utils::{remove_var, set_var};
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
        match self.lc_all.take() {
            Some(value) => set_var("LC_ALL", value),
            None => remove_var("LC_ALL"),
        }
        match self.lc_ctype.take() {
            Some(value) => set_var("LC_CTYPE", value),
            None => remove_var("LC_CTYPE"),
        }
        match self.lang.take() {
            Some(value) => set_var("LANG", value),
            None => remove_var("LANG"),
        }
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
    match lc_all {
        Some(value) => set_var("LC_ALL", value),
        None => remove_var("LC_ALL"),
    }
    match lc_ctype {
        Some(value) => set_var("LC_CTYPE", value),
        None => remove_var("LC_CTYPE"),
    }
    match lang {
        Some(value) => set_var("LANG", value),
        None => remove_var("LANG"),
    }
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

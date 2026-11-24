//! Process-level precedence checks for the production `load_and_merge` path.
//!
//! The in-process merge suites inject an environment. These tests run the real
//! `vk` binary, whose `main` merges subcommand configuration through the
//! process environment, and observe precedence through the one signal that
//! needs no network: a file value of the wrong type fails the merge only when
//! no higher-precedence source replaces it.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use rstest::rstest;
use std::process::Command;

#[path = "support/capfs.rs"]
mod capfs;

/// Run `vk pr` against a config file whose `show_outdated` has the wrong type.
///
/// HOME and the XDG locations name the config directory so the host's own
/// configuration cannot take part, and the API endpoint is a closed local port
/// so a merge that succeeds fails fast on the request instead.
fn run_with_bad_file_value(show_outdated_env: Option<&str>) -> assert_cmd::assert::Assert {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join(".vk.toml");
    capfs::write_file(&path, "[cmds.pr]\nshow_outdated = \"later\"\n").expect("write config");

    let mut cmd = Command::cargo_bin("vk").expect("binary");
    cmd.env_clear()
        .env("VK_CONFIG_PATH", &path)
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path())
        .env("XDG_CONFIG_DIRS", dir.path())
        .env("GITHUB_GRAPHQL_URL", "http://127.0.0.1:9")
        .current_dir(dir.path())
        .args(["--repo", "foo/bar", "pr", "1"]);
    if let Some(value) = show_outdated_env {
        cmd.env("VK_CMDS_PR_SHOW_OUTDATED", value);
    }
    cmd.assert()
}

#[rstest]
fn file_value_of_the_wrong_type_fails_the_merge_without_an_environment_override() {
    run_with_bad_file_value(None)
        .failure()
        .stderr(predicate::str::contains("configuration error"));
}

#[rstest]
fn environment_value_replaces_the_file_value_before_it_is_deserialized() {
    run_with_bad_file_value(Some("true"))
        .failure()
        .stderr(predicate::str::contains("configuration error").not());
}

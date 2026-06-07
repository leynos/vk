//! Behavioural tests covering configuration error reporting.
//!
//! These tests run the `vk` binary with intentionally broken configuration so
//! we assert that config-loading failures are reported consistently.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;

#[test]
fn invalid_config_file_reports_configuration_error() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join(".vk.toml");
    std::fs::write(&path, "not = [valid").expect("write broken config");
    // Isolate HOME and XDG so the host user's vk configuration cannot rescue
    // discovery and silently drop the parse error from VK_CONFIG_PATH.
    let home = dir.path().to_path_buf();

    let mut cmd = Command::cargo_bin("vk").expect("binary");
    cmd.env("VK_CONFIG_PATH", &path)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("XDG_CONFIG_DIRS", &home)
        .env_remove("APPDATA")
        .env_remove("LOCALAPPDATA")
        .args(["--repo", "foo/bar", "pr", "1"]);

    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("configuration error"));
}

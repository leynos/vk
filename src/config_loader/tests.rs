//! Tests for configuration loading.

use super::{
    EXPLICIT_CONFIG_PATH_ENV, explicit_config_path,
    load_global_args_without_cli_overrides_from_process_args, validate_explicit_config_path,
};
use crate::cli_args::GlobalArgs;
use ortho_config::MapEnv;
use ortho_config::{EnvSource, ScanEnvSource};
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

/// Build an injected environment that points every discovery root at `dir`.
fn sandbox_env(dir: &Path) -> MapEnv {
    [
        "APPDATA",
        "LOCALAPPDATA",
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_CONFIG_DIRS",
    ]
    .into_iter()
    .fold(MapEnv::new().with_current_dir(dir), |env, key| {
        env.with_var(key, dir)
    })
}

/// Load the global arguments for a bare `vk` invocation from `env`.
fn load_bare_vk<E>(env: E) -> ortho_config::OrthoResult<GlobalArgs>
where
    E: EnvSource + ScanEnvSource + 'static,
{
    load_global_args_without_cli_overrides_from_process_args([OsString::from("vk")], &Arc::new(env))
}

/// Load a bare `vk` invocation in a fresh sandbox whose selector is `selector`.
fn load_with_selector(selector: &str) -> GlobalArgs {
    let dir = tempfile::tempdir().expect("create config sandbox");
    crate::fs_access::write_string(&dir.path().join("rel.toml"), "repo = \"from-relative\"\n")
        .expect("write config");
    load_bare_vk(sandbox_env(dir.path()).with_var(EXPLICIT_CONFIG_PATH_ENV, selector))
        .expect("load global args")
}

fn setup_global_args_without_cli_overrides<I, F>(configure: F) -> (tempfile::TempDir, GlobalArgs)
where
    I: IntoIterator<Item = OsString>,
    F: FnOnce(&Path) -> I,
{
    let dir = tempfile::tempdir().expect("create config sandbox");
    let env = Arc::new(sandbox_env(dir.path()));
    let global =
        load_global_args_without_cli_overrides_from_process_args(configure(dir.path()), &env)
            .expect("load global args");
    (dir, global)
}

#[test]
fn load_global_args_without_cli_overrides_defaults_cleanly() {
    let (_sandbox, global) = setup_global_args_without_cli_overrides(|_| [OsString::from("vk")]);
    assert!(global.repo.is_none());
    assert!(global.github_token.is_none());
    assert!(global.transcript.is_none());
    assert!(global.http_timeout.is_none());
    assert!(global.connect_timeout.is_none());
}

#[test]
fn load_global_args_without_cli_overrides_honours_config_path_override() {
    let (_sandbox, global) = setup_global_args_without_cli_overrides(|sandbox| {
        let config_path = sandbox.join("override.toml");
        crate::fs_access::write_string(&config_path, "repo = \"from-config-path\"\n")
            .expect("write config");

        [
            OsString::from("vk"),
            OsString::from("--config-path"),
            config_path.into_os_string(),
            OsString::from("pr"),
            OsString::from("1"),
        ]
    });

    assert_eq!(global.repo.as_deref(), Some("from-config-path"));
}

#[test]
fn load_global_args_without_cli_overrides_reports_broken_explicit_config() {
    let sandbox = tempfile::tempdir().expect("create config sandbox");
    let broken_path = sandbox.path().join("broken.toml");
    crate::fs_access::write_string(&broken_path, "not = [valid").expect("write broken config");
    let result =
        load_bare_vk(sandbox_env(sandbox.path()).with_var(EXPLICIT_CONFIG_PATH_ENV, &broken_path));

    let err = result.expect_err("broken explicit config must surface a configuration error");
    let rendered = format!("{err}");
    assert!(
        !rendered.is_empty(),
        "configuration error should describe the failure",
    );
}

#[test]
fn a_broken_relative_explicit_config_path_in_the_injected_directory_is_reported() {
    let sandbox = tempfile::tempdir().expect("create config sandbox");
    crate::fs_access::write_string(&sandbox.path().join("broken.toml"), "not = [valid")
        .expect("write broken config");

    let env = sandbox_env(sandbox.path()).with_var(EXPLICIT_CONFIG_PATH_ENV, "broken.toml");

    // Validation alone: a process-relative read would find no file and pass.
    validate_explicit_config_path(&env)
        .expect_err("the relative selector must be read from the injected directory");
}

/// A rooted path without a drive is left unanchored, even without a working directory.
#[cfg(windows)]
#[test]
fn a_rooted_explicit_config_path_without_a_drive_is_left_unchanged() {
    let rooted = PathBuf::from(r"\vk.toml");
    let with_dir = MapEnv::new()
        .with_current_dir("C:\\injected")
        .with_var(EXPLICIT_CONFIG_PATH_ENV, &rooted);
    let without_dir = NoCurrentDir(MapEnv::new().with_var(EXPLICIT_CONFIG_PATH_ENV, &rooted));

    assert_eq!(
        explicit_config_path(&with_dir),
        Some(rooted.clone()),
        "a drive-less rooted selector must not be anchored to the injected directory",
    );
    assert_eq!(
        explicit_config_path(&without_dir),
        Some(rooted),
        "a drive-less rooted selector must survive an unreadable working directory",
    );
}

/// An environment whose working directory cannot be read.
#[derive(Debug)]
struct NoCurrentDir(MapEnv);

impl EnvSource for NoCurrentDir {
    fn get(&self, key: &str) -> Option<OsString> {
        self.0.get(key)
    }

    fn current_dir(&self) -> std::io::Result<PathBuf> {
        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
    }
}

impl ScanEnvSource for NoCurrentDir {
    fn scan(&self) -> Vec<(OsString, OsString)> {
        self.0.scan()
    }
}

#[test]
fn a_relative_explicit_config_path_does_not_fail_loading_without_a_working_directory() {
    let dir = tempfile::tempdir().expect("create config sandbox");
    let env = NoCurrentDir(sandbox_env(dir.path()).with_var(EXPLICIT_CONFIG_PATH_ENV, "vk.toml"));

    let global = load_bare_vk(env).expect("the selector is dropped, as in discovery");

    assert!(
        global.repo.is_none(),
        "a dropped selector must leave the defaults untouched",
    );
}

#[test]
fn a_relative_explicit_config_path_is_anchored_to_the_injected_directory() {
    let dir = Path::new("/injected/dir");
    let env = MapEnv::new()
        .with_current_dir(dir)
        .with_var(EXPLICIT_CONFIG_PATH_ENV, "vk.toml");

    assert_eq!(
        explicit_config_path(&env),
        Some(dir.join("vk.toml")),
        "a relative selector must be anchored to the injected directory",
    );
}

#[test]
fn an_absolute_explicit_config_path_is_left_unchanged() {
    let absolute = std::env::temp_dir().join("vk.toml");
    let env = MapEnv::new()
        .with_current_dir("/injected/dir")
        .with_var(EXPLICIT_CONFIG_PATH_ENV, &absolute);

    assert_eq!(
        explicit_config_path(&env),
        Some(absolute),
        "an absolute selector must not be re-anchored",
    );
}

#[test]
fn an_empty_explicit_config_path_selects_nothing() {
    let env = MapEnv::new()
        .with_current_dir("/injected/dir")
        .with_var(EXPLICIT_CONFIG_PATH_ENV, "");

    assert_eq!(
        explicit_config_path(&env),
        None,
        "an empty selector must select nothing",
    );
}

#[test]
fn a_relative_explicit_config_path_selects_nothing_without_a_working_directory() {
    let env = NoCurrentDir(MapEnv::new().with_var(EXPLICIT_CONFIG_PATH_ENV, "vk.toml"));

    assert_eq!(
        explicit_config_path(&env),
        None,
        "a relative selector must be dropped when the working directory is unreadable",
    );
}

#[test]
fn an_empty_explicit_config_path_does_not_fail_loading() {
    assert!(
        load_with_selector("").repo.is_none(),
        "an empty selector is ignored"
    );
}

#[test]
fn a_relative_explicit_config_path_loads_from_the_injected_directory() {
    assert_eq!(
        load_with_selector("rel.toml").repo.as_deref(),
        Some("from-relative"),
        "the relative selector must load from the injected directory",
    );
}

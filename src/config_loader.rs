//! Helpers for loading global configuration without subcommand CLI overrides.
//!
//! The binary parses full CLI input, but the global configuration flatten group
//! must be merged separately so subcommand tokens do not interfere with config
//! discovery. This module preserves generated discovery flags such as
//! `--config-path` while filtering the empty CLI layer that would otherwise
//! flatten grouped global values to `null`.
//!
//! It also surfaces parse errors for the explicit `VK_CONFIG_PATH` file. The
//! `ortho_config` discovery pipeline treats env-provided paths as optional and
//! silently swallows broken candidates when any other layer (such as
//! `~/.config/vk/config.toml`) loads successfully. When the user has set
//! `VK_CONFIG_PATH` explicitly the intent is unambiguous, so we validate that
//! file up-front and refuse to proceed if it fails to parse.

use crate::cli_args::GlobalArgs;
use ortho_config::{
    EnvSource, OrthoJsonMergeExt, ProcessEnv, ScanEnvSource,
    declarative::{LayerComposition, MergeLayer, MergeProvenance},
};
use std::borrow::Cow;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

/// Environment variable that selects an explicit configuration file.
const EXPLICIT_CONFIG_PATH_ENV: &str = "VK_CONFIG_PATH";

/// Load global configuration layers without letting an empty CLI flatten group
/// overwrite file or environment values.
pub(crate) fn load_global_args_without_cli_overrides() -> ortho_config::OrthoResult<GlobalArgs> {
    load_global_args_without_cli_overrides_from_process_args(
        std::env::args_os(),
        &Arc::new(ProcessEnv),
    )
}

/// Validate the file referenced by `VK_CONFIG_PATH`, if any is set.
///
/// Returns `Ok(())` when the variable is unset, empty, or points at a missing
/// file. Returns the parse error when the file exists but cannot be parsed,
/// so a misconfigured user-provided path surfaces a clear "configuration
/// error" instead of being silently dropped by discovery's optional-layer
/// fallback.
fn validate_explicit_config_path(env: &dyn EnvSource) -> ortho_config::OrthoResult<()> {
    let Some(path) = explicit_config_path(env) else {
        return Ok(());
    };
    // `load_config_file` returns `Ok(None)` when the file does not exist; we
    // tolerate that case to mirror ortho_config's existing semantics. Parse
    // failures, however, become `Err` and propagate.
    ortho_config::file::load_config_file(&path)?;
    Ok(())
}

/// Return the path discovery will use for `VK_CONFIG_PATH`, if any.
///
/// An unset or empty value selects nothing. A relative value is resolved
/// against the environment's working directory, as discovery does; when that
/// directory cannot be read discovery drops the selector, so nothing is
/// validated rather than reading a process-relative file.
fn explicit_config_path(env: &dyn EnvSource) -> Option<PathBuf> {
    let raw = env
        .get(EXPLICIT_CONFIG_PATH_ENV)
        .filter(|raw| !raw.is_empty())?;
    let path = PathBuf::from(raw);
    // A rooted path with no drive (`\config.toml` on Windows) names a fixed
    // location on the current drive; discovery leaves it alone, so do we.
    if path.is_absolute() || path.has_root() {
        return Some(path);
    }
    env.current_dir().ok().map(|dir| dir.join(path))
}

/// Preserve generated discovery overrides from the process argv while
/// excluding subcommand tokens that `GlobalArgs` cannot parse on its own.
fn load_global_args_without_cli_overrides_from_process_args<I, T, E>(
    args: I,
    env: &Arc<E>,
) -> ortho_config::OrthoResult<GlobalArgs>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    E: EnvSource + ScanEnvSource + 'static,
{
    load_global_args_without_cli_overrides_from_iter(global_discovery_args_from_iter(args), env)
}

/// Compose and merge global configuration layers while filtering out an empty
/// CLI layer that would otherwise flatten grouped values to `null`.
fn load_global_args_without_cli_overrides_from_iter<I, T, E>(
    args: I,
    env: &Arc<E>,
) -> ortho_config::OrthoResult<GlobalArgs>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    E: EnvSource + ScanEnvSource + 'static,
{
    validate_explicit_config_path(env.as_ref())?;
    let composition = GlobalArgs::compose_layers_from_iter_with_env(args, Arc::clone(env));
    let (layers, errors) = composition.into_parts();
    let mut filtered_layers = Vec::with_capacity(layers.len() + 1);
    let default_globals = serde_json::to_value(GlobalArgs::default()).into_ortho_merge_json()?;
    filtered_layers.push(MergeLayer::defaults(Cow::Owned(default_globals)));

    for layer in layers {
        if layer.provenance() == MergeProvenance::Cli {
            let value = layer.into_value();
            if value.is_null() {
                continue;
            }
            filtered_layers.push(MergeLayer::cli(Cow::Owned(value)));
        } else {
            filtered_layers.push(layer);
        }
    }

    LayerComposition::new(filtered_layers, errors).into_merge_result(GlobalArgs::merge_from_layers)
}

/// Extract the generated `--config-path` discovery override from full process
/// argv so the global loader can honour explicit config-file selection without
/// trying to parse subcommand tokens.
fn global_discovery_args_from_iter<I, T>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let mut args = args.into_iter();
    let mut filtered = vec![args.next().map_or_else(|| OsString::from("vk"), Into::into)];

    while let Some(raw_arg) = args.next() {
        let arg = raw_arg.into();
        if arg == "--config-path" {
            filtered.push(arg);
            if let Some(value) = args.next() {
                filtered.push(value.into());
            }
            continue;
        }

        let Some(arg_str) = arg.to_str() else {
            continue;
        };
        if arg_str.starts_with("--config-path=") {
            filtered.push(arg);
        }
    }

    filtered
}

#[cfg(test)]
mod tests {
    //! Tests for configuration loading.

    use super::{
        EXPLICIT_CONFIG_PATH_ENV, explicit_config_path,
        load_global_args_without_cli_overrides_from_process_args,
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
        load_global_args_without_cli_overrides_from_process_args(
            [OsString::from("vk")],
            &Arc::new(env),
        )
    }

    /// Load a bare `vk` invocation in a fresh sandbox whose selector is `selector`.
    fn load_with_selector(selector: &str) -> GlobalArgs {
        let dir = tempfile::tempdir().expect("create config sandbox");
        crate::fs_access::write_string(&dir.path().join("rel.toml"), "repo = \"from-relative\"\n")
            .expect("write config");
        load_bare_vk(sandbox_env(dir.path()).with_var(EXPLICIT_CONFIG_PATH_ENV, selector))
            .expect("load global args")
    }

    fn setup_global_args_without_cli_overrides<I, F>(
        configure: F,
    ) -> (tempfile::TempDir, GlobalArgs)
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
        let (_sandbox, global) =
            setup_global_args_without_cli_overrides(|_| [OsString::from("vk")]);
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
        let result = load_bare_vk(
            sandbox_env(sandbox.path()).with_var(EXPLICIT_CONFIG_PATH_ENV, &broken_path),
        );

        let err = result.expect_err("broken explicit config must surface a configuration error");
        let rendered = format!("{err}");
        assert!(
            !rendered.is_empty(),
            "configuration error should describe the failure",
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
        let env =
            NoCurrentDir(sandbox_env(dir.path()).with_var(EXPLICIT_CONFIG_PATH_ENV, "vk.toml"));

        let global = load_bare_vk(env).expect("the selector is dropped, as in discovery");

        assert!(global.repo.is_none());
    }

    #[test]
    fn a_relative_explicit_config_path_is_anchored_to_the_injected_directory() {
        let dir = Path::new("/injected/dir");
        let env = MapEnv::new()
            .with_current_dir(dir)
            .with_var(EXPLICIT_CONFIG_PATH_ENV, "vk.toml");

        assert_eq!(explicit_config_path(&env), Some(dir.join("vk.toml")));
    }

    #[test]
    fn an_absolute_explicit_config_path_is_left_unchanged() {
        let absolute = std::env::temp_dir().join("vk.toml");
        let env = MapEnv::new()
            .with_current_dir("/injected/dir")
            .with_var(EXPLICIT_CONFIG_PATH_ENV, &absolute);

        assert_eq!(explicit_config_path(&env), Some(absolute));
    }

    #[test]
    fn an_empty_explicit_config_path_selects_nothing() {
        let env = MapEnv::new()
            .with_current_dir("/injected/dir")
            .with_var(EXPLICIT_CONFIG_PATH_ENV, "");

        assert_eq!(explicit_config_path(&env), None);
    }

    #[test]
    fn a_relative_explicit_config_path_selects_nothing_without_a_working_directory() {
        let env = NoCurrentDir(MapEnv::new().with_var(EXPLICIT_CONFIG_PATH_ENV, "vk.toml"));

        assert_eq!(explicit_config_path(&env), None);
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
            Some("from-relative")
        );
    }
}

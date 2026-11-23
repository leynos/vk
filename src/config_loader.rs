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
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
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
    let anchored = env
        .get(EXPLICIT_CONFIG_PATH_ENV)
        .map(|raw| anchor_to_current_dir(env, raw));
    validate_explicit_config_path_value(anchored.as_deref())
}

/// Resolve a relative path against the environment's working directory, as
/// discovery does, so validation and discovery select the same file.
fn anchor_to_current_dir(env: &dyn EnvSource, raw: OsString) -> OsString {
    if Path::new(&raw).is_absolute() {
        return raw;
    }
    let anchored = env.current_dir().ok().map(|dir| dir.join(&raw));
    anchored.map_or(raw, PathBuf::into_os_string)
}

/// Validate an optional explicit configuration path value.
fn validate_explicit_config_path_value(raw: Option<&OsStr>) -> ortho_config::OrthoResult<()> {
    let Some(raw) = raw else {
        return Ok(());
    };
    if raw.is_empty() {
        return Ok(());
    }
    // `load_config_file` returns `Ok(None)` when the file does not exist; we
    // tolerate that case to mirror ortho_config's existing semantics. Parse
    // failures, however, become `Err` and propagate.
    ortho_config::file::load_config_file(Path::new(raw))?;
    Ok(())
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
        EXPLICIT_CONFIG_PATH_ENV, anchor_to_current_dir,
        load_global_args_without_cli_overrides_from_process_args,
    };
    use crate::cli_args::GlobalArgs;
    use ortho_config::MapEnv;
    use std::ffi::OsString;
    use std::path::Path;
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
        let env =
            Arc::new(sandbox_env(sandbox.path()).with_var(EXPLICIT_CONFIG_PATH_ENV, &broken_path));

        let result =
            load_global_args_without_cli_overrides_from_process_args([OsString::from("vk")], &env);

        let err = result.expect_err("broken explicit config must surface a configuration error");
        let rendered = format!("{err}");
        assert!(
            !rendered.is_empty(),
            "configuration error should describe the failure",
        );
    }

    #[test]
    fn a_relative_explicit_config_path_is_anchored_to_the_injected_directory() {
        let dir = Path::new("/injected/dir");
        let env = MapEnv::new().with_current_dir(dir);

        let anchored = anchor_to_current_dir(&env, OsString::from("vk.toml"));

        assert_eq!(Path::new(&anchored), dir.join("vk.toml"));
    }

    #[test]
    fn an_absolute_explicit_config_path_is_left_unchanged() {
        let absolute = std::env::temp_dir().join("vk.toml");
        let env = MapEnv::new().with_current_dir("/injected/dir");

        let anchored = anchor_to_current_dir(&env, absolute.clone().into_os_string());

        assert_eq!(Path::new(&anchored), absolute);
    }
}

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
    OrthoJsonMergeExt,
    declarative::{LayerComposition, MergeLayer, MergeProvenance},
};
use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::path::Path;

/// Environment variable that selects an explicit configuration file.
const EXPLICIT_CONFIG_PATH_ENV: &str = "VK_CONFIG_PATH";

/// Load global configuration layers without letting an empty CLI flatten group
/// overwrite file or environment values.
pub(crate) fn load_global_args_without_cli_overrides() -> ortho_config::OrthoResult<GlobalArgs> {
    load_global_args_without_cli_overrides_from_process_args(std::env::args_os())
}

/// Validate the file referenced by `VK_CONFIG_PATH`, if any is set.
///
/// Returns `Ok(())` when the variable is unset, empty, or points at a missing
/// file. Returns the parse error when the file exists but cannot be parsed,
/// so a misconfigured user-provided path surfaces a clear "configuration
/// error" instead of being silently dropped by discovery's optional-layer
/// fallback.
fn validate_explicit_config_path() -> ortho_config::OrthoResult<()> {
    validate_explicit_config_path_value(std::env::var_os(EXPLICIT_CONFIG_PATH_ENV).as_deref())
}

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
fn load_global_args_without_cli_overrides_from_process_args<I, T>(
    args: I,
) -> ortho_config::OrthoResult<GlobalArgs>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    load_global_args_without_cli_overrides_from_iter(global_discovery_args_from_iter(args))
}

/// Compose and merge global configuration layers while filtering out an empty
/// CLI layer that would otherwise flatten grouped values to `null`.
fn load_global_args_without_cli_overrides_from_iter<I, T>(
    args: I,
) -> ortho_config::OrthoResult<GlobalArgs>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    validate_explicit_config_path()?;
    let composition = GlobalArgs::compose_layers_from_iter(args);
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
    let mut has_config_path = false;

    while let Some(raw_arg) = args.next() {
        let arg = raw_arg.into();
        if arg == "--config-path" {
            has_config_path = true;
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
            has_config_path = true;
            filtered.push(arg);
        }
    }

    if !has_config_path {
        append_config_path_env_override(&mut filtered);
    }

    filtered
}

fn append_config_path_env_override(filtered: &mut Vec<OsString>) {
    let config_path =
        std::env::var_os("VK_CONFIG_PATH").or_else(|| std::env::var_os("CONFIG_PATH"));

    if let Some(config_path) = config_path {
        filtered.push(OsString::from("--config-path"));
        filtered.push(config_path);
    }
}
mod tests {
    fn setup_global_args_without_cli_overrides<I, F>(configure: F) -> (EnvSandbox, GlobalArgs)
    where
        I: IntoIterator<Item = OsString>,
        F: FnOnce(&EnvSandbox) -> I,
    {
        let sandbox = EnvSandbox::new().expect("create config sandbox");
        let global = load_global_args_without_cli_overrides_from_process_args(configure(&sandbox))
            .expect("load global args");
        (sandbox, global)
    }

    #[test]
    #[serial]
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
    #[serial]
    fn load_global_args_without_cli_overrides_honours_config_path_override() {
        let (_sandbox, global) = setup_global_args_without_cli_overrides(|sandbox| {
            let config_path = sandbox.path().join("override.toml");
            std::fs::write(&config_path, "repo = \"from-config-path\"\n").expect("write config");

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
    #[serial]
    fn load_global_args_without_cli_overrides_reports_broken_explicit_config() {
        let sandbox = EnvSandbox::new().expect("create config sandbox");
        let broken_path = sandbox.path().join("broken.toml");
        std::fs::write(&broken_path, "not = [valid").expect("write broken config");
        environment::set_var(EXPLICIT_CONFIG_PATH_ENV, &broken_path);

        let result =
            load_global_args_without_cli_overrides_from_process_args([OsString::from("vk")]);

        let err = result.expect_err("broken explicit config must surface a configuration error");
        let rendered = format!("{err}");
        assert!(
            !rendered.is_empty(),
            "configuration error should describe the failure",
        );
        // `EnvSandbox` restores VK_CONFIG_PATH on drop.
        drop(sandbox);
    }

    #[test]
    #[serial]
    fn load_global_args_without_cli_overrides_honours_config_path_env() {
        let (_sandbox, global) = setup_global_args_without_cli_overrides(|sandbox| {
            let config_path = sandbox.path().join("env-override.toml");
            std::fs::write(&config_path, "repo = \"from-env-config-path\"\n")
                .expect("write config");
            // SAFETY: `EnvSandbox` holds the shared environment sandbox lock.
            unsafe { env::set_var("VK_CONFIG_PATH", config_path) };

            [
                OsString::from("vk"),
                OsString::from("pr"),
                OsString::from("1"),
            ]
        });

        assert_eq!(global.repo.as_deref(), Some("from-env-config-path"));
    }
}

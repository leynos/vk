//! Helpers for loading global configuration without subcommand CLI overrides.
//!
//! The binary parses full CLI input, but the global configuration flatten group
//! must be merged separately so subcommand tokens do not interfere with config
//! discovery. This module preserves generated discovery flags such as
//! `--config-path` while filtering the empty CLI layer that would otherwise
//! flatten grouped global values to `null`.

use crate::cli_args::GlobalArgs;
use ortho_config::{
    OrthoJsonMergeExt,
    declarative::{LayerComposition, MergeLayer, MergeProvenance},
};
use std::borrow::Cow;
use std::ffi::OsString;

/// Load global configuration layers without letting an empty CLI flatten group
/// overwrite file or environment values.
pub(crate) fn load_global_args_without_cli_overrides() -> ortho_config::OrthoResult<GlobalArgs> {
    load_global_args_without_cli_overrides_from_process_args(std::env::args_os())
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
    use super::{
        load_global_args_without_cli_overrides_from_iter,
        load_global_args_without_cli_overrides_from_process_args,
    };
    use crate::test_utils::{remove_var, restore_optional_env, set_var};
    use serial_test::serial;
    use std::ffi::OsString;
    use vk::environment;

    #[test]
    #[serial]
    fn load_global_args_without_cli_overrides_defaults_cleanly() {
        let config_sandbox = tempfile::tempdir().expect("create config sandbox");
        let sandbox = config_sandbox.path().to_string_lossy().into_owned();
        let old_repo = environment::var("VK_REPO").ok();
        let old_token = environment::var("VK_GITHUB_TOKEN").ok();
        let old_config_path = environment::var("VK_CONFIG_PATH").ok();
        let old_home = environment::var("HOME").ok();
        let old_xdg_config_home = environment::var("XDG_CONFIG_HOME").ok();
        let old_xdg_config_dirs = environment::var("XDG_CONFIG_DIRS").ok();
        remove_var("VK_REPO");
        remove_var("VK_GITHUB_TOKEN");
        remove_var("VK_CONFIG_PATH");
        set_var("HOME", &sandbox);
        set_var("XDG_CONFIG_HOME", &sandbox);
        set_var("XDG_CONFIG_DIRS", &sandbox);

        let global = load_global_args_without_cli_overrides_from_iter([OsString::from("vk")])
            .expect("load global args");
        assert!(global.repo.is_none());
        assert!(global.github_token.is_none());
        assert!(global.transcript.is_none());
        assert!(global.http_timeout.is_none());
        assert!(global.connect_timeout.is_none());

        match old_repo {
            Some(value) => set_var("VK_REPO", value),
            None => remove_var("VK_REPO"),
        }
        match old_token {
            Some(value) => set_var("VK_GITHUB_TOKEN", value),
            None => remove_var("VK_GITHUB_TOKEN"),
        }
        restore_optional_env("VK_CONFIG_PATH", old_config_path);
        restore_optional_env("HOME", old_home);
        restore_optional_env("XDG_CONFIG_HOME", old_xdg_config_home);
        restore_optional_env("XDG_CONFIG_DIRS", old_xdg_config_dirs);
    }

    #[test]
    #[serial]
    fn load_global_args_without_cli_overrides_honours_config_path_override() {
        let config_sandbox = tempfile::tempdir().expect("create config sandbox");
        let sandbox = config_sandbox.path().to_string_lossy().into_owned();
        let config_path = config_sandbox.path().join("override.toml");
        std::fs::write(&config_path, "repo = \"from-config-path\"\n").expect("write config");

        let old_repo = environment::var("VK_REPO").ok();
        let old_token = environment::var("VK_GITHUB_TOKEN").ok();
        let old_config_path = environment::var("VK_CONFIG_PATH").ok();
        let old_home = environment::var("HOME").ok();
        let old_xdg_config_home = environment::var("XDG_CONFIG_HOME").ok();
        let old_xdg_config_dirs = environment::var("XDG_CONFIG_DIRS").ok();
        remove_var("VK_REPO");
        remove_var("VK_GITHUB_TOKEN");
        remove_var("VK_CONFIG_PATH");
        set_var("HOME", &sandbox);
        set_var("XDG_CONFIG_HOME", &sandbox);
        set_var("XDG_CONFIG_DIRS", &sandbox);

        let global = load_global_args_without_cli_overrides_from_process_args([
            OsString::from("vk"),
            OsString::from("--config-path"),
            config_path.into_os_string(),
            OsString::from("pr"),
            OsString::from("1"),
        ])
        .expect("load global args");

        assert_eq!(global.repo.as_deref(), Some("from-config-path"));

        match old_repo {
            Some(value) => set_var("VK_REPO", value),
            None => remove_var("VK_REPO"),
        }
        match old_token {
            Some(value) => set_var("VK_GITHUB_TOKEN", value),
            None => remove_var("VK_GITHUB_TOKEN"),
        }
        restore_optional_env("VK_CONFIG_PATH", old_config_path);
        restore_optional_env("HOME", old_home);
        restore_optional_env("XDG_CONFIG_HOME", old_xdg_config_home);
        restore_optional_env("XDG_CONFIG_DIRS", old_xdg_config_dirs);
    }
}

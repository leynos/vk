//! Subcommand configuration helpers for integration tests.
//!
//! Defines utilities that wrap CLI argument builders and merge logic so tests
//! can exercise configuration, environment, and CLI precedence.

use crate::env_support::{DirGuard, EnvGuard, setup_env_and_config};
use clap::CommandFactory;
use ortho_config::SubcmdConfigMerge;
use vk::cli_args::ResolveArgs;
use vk::test_utils::{remove_var, set_var};
use vk::{IssueArgs, PrArgs};

/// Merge CLI arguments against config and environment sources for tests.
pub fn merge_with_sources<T>(config: &str, env: &[(&str, Option<&str>)], cli: &T) -> T
where
    T: SubcmdConfigMerge + ortho_config::OrthoConfig + serde::Serialize + Default + CommandFactory,
{
    let mut keys: Vec<&str> = env.iter().map(|(key, _)| *key).collect();
    keys.push("VK_CONFIG_PATH");
    let _guard = EnvGuard::new(&keys);
    let (config_dir, _config_path) = setup_env_and_config(config);
    let _dir = DirGuard::enter(config_dir.path());

    for (key, value) in env {
        match value {
            Some(val) => set_var(key, val),
            None => remove_var(key),
        }
    }

    cli.load_and_merge()
        .unwrap_or_else(|err| panic!("merge {} args: {err}", std::any::type_name::<T>()))
}

/// Helper to build PR CLI arguments for tests.
pub fn pr_cli(reference: Option<&str>, files: &[&str]) -> PrArgs {
    let mut args = PrArgs::default();
    if let Some(reference) = reference {
        args.reference = Some(reference.to_owned());
    }
    if !files.is_empty() {
        args.files = files.iter().map(|value| (*value).to_owned()).collect();
    }
    args
}

/// Helper to build issue CLI arguments for tests.
pub fn issue_cli(reference: Option<&str>) -> IssueArgs {
    let mut args = IssueArgs::default();
    if let Some(reference) = reference {
        args.reference = Some(reference.to_owned());
    }
    args
}

/// Helper to build resolve CLI arguments for tests.
pub fn resolve_cli(reference: &str, message: Option<&str>) -> ResolveArgs {
    ResolveArgs {
        reference: reference.to_owned(),
        message: message.map(str::to_owned),
    }
}

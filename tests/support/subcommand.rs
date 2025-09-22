//! Subcommand configuration helpers for integration tests.
//!
//! Defines utilities that wrap CLI argument builders and merge logic so tests
//! can exercise configuration, environment, and CLI precedence.

use crate::env_support::{DirGuard, EnvGuard, setup_env_and_config};
use clap::CommandFactory;
use ortho_config::SubcmdConfigMerge;
use vk::test_utils::{remove_var, set_var};

/// Merge CLI arguments against config and environment sources for tests.
pub fn merge_with_sources<T>(config: &str, env: &[(&str, Option<&str>)], cli: &T) -> T
where
    T: SubcmdConfigMerge + ortho_config::OrthoConfig + serde::Serialize + Default + CommandFactory,
{
    let mut keys: Vec<&str> = env.iter().map(|(key, _)| *key).collect();
    keys.push("VK_CONFIG_PATH");
    let _guard = EnvGuard::new(&keys);
    let (config_dir, config_path) = setup_env_and_config(config);
    let _dir = DirGuard::enter(config_dir.path());

    for (key, value) in env {
        match value {
            Some(val) => set_var(key, val),
            None => remove_var(key),
        }
    }

    cli.load_and_merge().unwrap_or_else(|err| {
        panic!(
            "merge {} args with config {}: {err}",
            std::any::type_name::<T>(),
            config_path.display()
        )
    })
}

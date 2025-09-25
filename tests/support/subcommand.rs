//! Subcommand configuration helpers for integration tests.
//!
//! Defines utilities that wrap CLI argument builders and merge logic so tests
//! can exercise configuration, environment, and CLI precedence.

use crate::env_support::{EnvGuard, maybe_enter_dir, setup_env_and_config};
use crate::merge_support::environment_keys;
use ortho_config::SubcmdConfigMerge;
use vk::test_utils::{remove_var, set_var};

/// Merge CLI arguments against config and environment sources for tests.
///
/// # Examples
/// ```rust,ignore
/// use vk::cli_args::PrArgs;
/// use crate::sub_support::merge_with_sources;
///
/// let cli = PrArgs::default();
/// let merged = merge_with_sources(
///     "[cmds.pr]\nreference = \"ref\"",
///     &[("VKCMDS_PR_REFERENCE", Some("env_ref"))],
///     true,
///     &cli,
/// );
/// assert_eq!(merged.reference.as_deref(), Some("env_ref"));
/// ```
pub fn merge_with_sources<T>(
    config: &str,
    env: &[(&str, Option<&str>)],
    enter_config_dir: bool,
    cli: &T,
) -> T
where
    // SubcmdConfigMerge::load_and_merge requires Default on implementors.
    T: SubcmdConfigMerge + ortho_config::OrthoConfig + serde::Serialize + Default,
{
    let keys = environment_keys(env);
    let _guard = EnvGuard::new(&keys);
    let (config_dir, config_path) = setup_env_and_config(config);
    let _dir = maybe_enter_dir(enter_config_dir, config_dir.path());

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

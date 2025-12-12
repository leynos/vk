//! Subcommand configuration helpers for integration tests.
//!
//! Defines utilities that wrap CLI argument builders and merge logic so tests
//! can exercise configuration, environment, and CLI precedence.

use crate::env_support::{EnvGuard, maybe_enter_dir, setup_env_and_config};
use crate::merge_support::environment_keys;
use ortho_config::SubcmdConfigMerge;

/// Merge CLI arguments against config and environment sources for tests.
///
/// Arguments:
/// - `config`: TOML written to a temporary config file referenced by `VK_CONFIG_PATH`.
/// - `env`: environment variable pairs; `Some` sets values and `None` removes them before merging.
/// - `enter_config_dir`: when true, temporarily changes into the config directory for path-relative merges.
/// - `cli`: CLI arguments providing highest-precedence overrides.
///
/// # Examples
/// ```rust,ignore
/// use vk::cli_args::PrArgs;
/// use crate::sub_support::merge_with_sources;
///
/// let cli = PrArgs::default();
/// let merged = merge_with_sources(
///     "[cmds.pr]\nreference = \"ref\"",
///     &[("VK_CMDS_PR_REFERENCE", Some("env_ref"))],
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
    T: SubcmdConfigMerge + ortho_config::OrthoConfig + serde::Serialize,
    T: Default,
{
    let keys = environment_keys(env);
    let _guard = EnvGuard::new(&keys);
    let (config_dir, config_path) = setup_env_and_config(config);
    let _dir = maybe_enter_dir(enter_config_dir, config_dir.path());

    crate::env_support::apply_env(env);

    let context = format!(
        "merge {} args with config {}",
        std::any::type_name::<T>(),
        config_path.display()
    );
    cli.load_and_merge().expect(&context)
}

//! Subcommand configuration helpers for integration tests.
//!
//! Defines utilities that wrap CLI argument builders and merge logic so tests
//! can exercise configuration, environment, and CLI precedence.

use crate::env_support::MergeEnv;
use ortho_config::SubcmdConfigMerge;

/// Merge CLI arguments against config and environment sources for tests.
///
/// Arguments:
/// - `config`: TOML written to a temporary config file referenced by `VK_CONFIG_PATH`.
/// - `env`: environment variable pairs injected into the merge; `Some` sets a value and `None` leaves it unset.
/// - `enter_config_dir`: when true, the injected working directory is the config directory, for path-relative merges.
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
    let merge_env = MergeEnv::new(config, env, enter_config_dir);

    let context = format!("merge {} args", std::any::type_name::<T>());
    cli.load_and_merge_with_env(merge_env.env())
        .expect(&context)
}

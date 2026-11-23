//! Authentication helpers for GitHub token resolution.
//!
//! Token resolution prefers the CLI flag, then `VK_GITHUB_TOKEN`, then
//! `GITHUB_TOKEN`, and finally configuration file values. Empty values are
//! ignored.

use ortho_config::{EnvSource, ProcessEnv};

/// Resolve the GitHub token from CLI, environment, and configuration inputs.
///
/// Precedence is:
/// - CLI flag
/// - `VK_GITHUB_TOKEN`
/// - `GITHUB_TOKEN`
/// - configuration file
///
/// Empty values are ignored. Returns an empty `String` when no source provides
/// a token.
///
/// # Examples
/// ```
/// use vk::resolve_github_token;
///
/// let token = resolve_github_token(Some("cli-token"), None);
/// assert_eq!(token, "cli-token");
/// ```
#[must_use]
pub fn resolve_github_token(cli_token: Option<&str>, config_token: Option<&str>) -> String {
    resolve_github_token_in(cli_token, config_token, &ProcessEnv)
}

/// Resolve the GitHub token, reading environment variables from `env`.
///
/// This is [`resolve_github_token`] with the environment injected, so tests
/// supply a map instead of changing the process environment.
pub(crate) fn resolve_github_token_in(
    cli_token: Option<&str>,
    config_token: Option<&str>,
    env: &dyn EnvSource,
) -> String {
    let cli_token = cli_token.filter(|token| !token.is_empty());
    cli_token
        .map(str::to_owned)
        .or_else(|| non_empty_var(env, "VK_GITHUB_TOKEN"))
        .or_else(|| non_empty_var(env, "GITHUB_TOKEN"))
        .or_else(|| {
            config_token
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

/// Read `key` from `env` as a non-empty Unicode string.
fn non_empty_var(env: &dyn EnvSource, key: &str) -> Option<String> {
    env.get(key)
        .and_then(|value| value.into_string().ok())
        .filter(|token| !token.is_empty())
}

#[cfg(test)]
mod tests {
    //! Tests for token resolution.

    use super::resolve_github_token_in;
    use ortho_config::MapEnv;
    use rstest::rstest;

    /// Build an environment holding only the token variables that are set.
    fn token_env(vk: Option<&str>, github: Option<&str>) -> MapEnv {
        [("VK_GITHUB_TOKEN", vk), ("GITHUB_TOKEN", github)]
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .fold(MapEnv::new(), |env, (key, value)| env.with_var(key, value))
    }

    #[rstest]
    #[case(
        Some("cli-token"),
        Some("vk-token"),
        Some("github-token"),
        Some("config-token"),
        "cli-token"
    )]
    #[case(
        None,
        Some("vk-token"),
        Some("github-token"),
        Some("config-token"),
        "vk-token"
    )]
    #[case(None, None, Some("github-token"), Some("config-token"), "github-token")]
    #[case(None, None, None, Some("config-token"), "config-token")]
    #[case(Some(""), Some(""), Some("github-token"), Some(""), "github-token")]
    fn resolve_github_token_cases(
        #[case] cli_token: Option<&str>,
        #[case] vk_env: Option<&'static str>,
        #[case] github_env: Option<&'static str>,
        #[case] config_token: Option<&str>,
        #[case] expected: &str,
    ) {
        let env = token_env(vk_env, github_env);
        assert_eq!(
            resolve_github_token_in(cli_token, config_token, &env),
            expected
        );
    }
}

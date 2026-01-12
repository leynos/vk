//! Authentication helpers for GitHub token resolution.
//!
//! Token resolution prefers the CLI flag, then `VK_GITHUB_TOKEN`, then
//! `GITHUB_TOKEN`, and finally configuration file values. Empty values are
//! ignored.

use vk::environment;

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
pub fn resolve_github_token(cli_token: Option<&str>, config_token: Option<&str>) -> String {
    let cli_token = cli_token.filter(|token| !token.is_empty());
    cli_token
        .map(str::to_owned)
        .or_else(|| {
            environment::var("VK_GITHUB_TOKEN")
                .ok()
                .filter(|token| !token.is_empty())
        })
        .or_else(|| {
            environment::var("GITHUB_TOKEN")
                .ok()
                .filter(|token| !token.is_empty())
        })
        .or_else(|| {
            config_token
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::resolve_github_token;
    use crate::test_utils::{apply_optional_env, restore_optional_env};
    use rstest::{fixture, rstest};
    use serial_test::serial;
    use vk::environment;

    struct TokenEnvGuard {
        old_vk: Option<String>,
        old_github: Option<String>,
    }

    impl Drop for TokenEnvGuard {
        fn drop(&mut self) {
            restore_optional_env("VK_GITHUB_TOKEN", self.old_vk.take());
            restore_optional_env("GITHUB_TOKEN", self.old_github.take());
        }
    }

    #[fixture]
    fn token_env() -> TokenEnvGuard {
        let old_vk = environment::var("VK_GITHUB_TOKEN").ok();
        let old_github = environment::var("GITHUB_TOKEN").ok();
        TokenEnvGuard { old_vk, old_github }
    }

    fn apply_token_env(vk: Option<&str>, github: Option<&str>) {
        apply_optional_env("VK_GITHUB_TOKEN", vk);
        apply_optional_env("GITHUB_TOKEN", github);
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
    #[serial]
    fn resolve_github_token_cases(
        #[case] cli_token: Option<&str>,
        #[case] vk_env: Option<&'static str>,
        #[case] github_env: Option<&'static str>,
        #[case] config_token: Option<&str>,
        #[case] expected: &str,
        token_env: TokenEnvGuard,
    ) {
        let _ = token_env;
        apply_token_env(vk_env, github_env);
        let config = config_token.map(str::to_string);
        assert_eq!(resolve_github_token(cli_token, config.as_deref()), expected);
    }
}

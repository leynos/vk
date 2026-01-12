//! Authentication helpers for GitHub token resolution.
//!
//! Token resolution prefers explicit configuration (CLI/config file), then
//! `VK_GITHUB_TOKEN`, and finally `GITHUB_TOKEN`. Empty values are ignored.

use crate::cli_args::GlobalArgs;
use vk::environment;

pub fn resolve_github_token(global: &GlobalArgs) -> String {
    global
        .github_token
        .as_deref()
        .filter(|token| !token.is_empty())
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
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::resolve_github_token;
    use crate::cli_args::GlobalArgs;
    use crate::test_utils::{remove_var, set_var};
    use serial_test::serial;
    use vk::environment;

    fn with_token_env<F>(vk: Option<&str>, github: Option<&str>, op: F)
    where
        F: FnOnce(),
    {
        let old_vk = environment::var("VK_GITHUB_TOKEN").ok();
        let old_github = environment::var("GITHUB_TOKEN").ok();

        match vk {
            Some(value) => set_var("VK_GITHUB_TOKEN", value),
            None => remove_var("VK_GITHUB_TOKEN"),
        }
        match github {
            Some(value) => set_var("GITHUB_TOKEN", value),
            None => remove_var("GITHUB_TOKEN"),
        }

        op();

        match old_vk {
            Some(value) => set_var("VK_GITHUB_TOKEN", value),
            None => remove_var("VK_GITHUB_TOKEN"),
        }
        match old_github {
            Some(value) => set_var("GITHUB_TOKEN", value),
            None => remove_var("GITHUB_TOKEN"),
        }
    }

    #[test]
    #[serial]
    fn resolve_github_token_prefers_global_value() {
        with_token_env(Some("env-token"), Some("github-token"), || {
            let global = GlobalArgs {
                github_token: Some("cli-token".to_string()),
                ..GlobalArgs::default()
            };

            assert_eq!(resolve_github_token(&global), "cli-token");
        });
    }

    #[test]
    #[serial]
    fn resolve_github_token_prefers_vk_environment() {
        with_token_env(Some("vk-token"), Some("github-token"), || {
            let global = GlobalArgs::default();

            assert_eq!(resolve_github_token(&global), "vk-token");
        });
    }

    #[test]
    #[serial]
    fn resolve_github_token_falls_back_to_github_token_env() {
        with_token_env(None, Some("github-token"), || {
            let global = GlobalArgs::default();

            assert_eq!(resolve_github_token(&global), "github-token");
        });
    }

    #[test]
    #[serial]
    fn resolve_github_token_ignores_empty_values() {
        with_token_env(Some(""), Some("github-token"), || {
            let global = GlobalArgs {
                github_token: Some(String::new()),
                ..GlobalArgs::default()
            };

            assert_eq!(resolve_github_token(&global), "github-token");
        });
    }
}

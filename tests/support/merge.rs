//! Shared helpers for merge configuration tests.
//!
//! Provides utilities for building owned strings from expected slices and
//! collecting environment variable keys to seed guards.

/// Convert string slices to owned [`String`] values so expectations can be compared.
///
/// # Examples
/// ```rust,ignore
/// use crate::merge_support::to_owned_vec;
/// let owned = to_owned_vec(&["first", "second"]);
/// assert_eq!(owned, vec![String::from("first"), String::from("second")]);
/// ```
pub fn to_owned_vec(values: &[&str]) -> Vec<String> {
    values.iter().map(|&value| value.to_owned()).collect()
}

/// Collect the environment variable keys used in `env`, ensuring `VK_CONFIG_PATH`
/// is present for guard setup.
///
/// Keys are sorted and deduplicated to support deterministic guard behaviour.
///
/// # Examples
/// ```rust,ignore
/// use crate::merge_support::environment_keys;
/// let keys = environment_keys(&[("FIRST", Some("value")), ("SECOND", None)]);
/// assert_eq!(keys, vec!["FIRST", "SECOND", "VK_CONFIG_PATH"]);
/// ```
pub fn environment_keys<'a>(env: &'a [(&'a str, Option<&'a str>)]) -> Vec<&'a str> {
    let mut keys: Vec<_> = env.iter().map(|(key, _)| *key).collect();
    if !keys.contains(&"VK_CONFIG_PATH") {
        keys.push("VK_CONFIG_PATH");
    }
    keys.sort_unstable();
    keys.dedup();
    keys
}

#[cfg(test)]
mod tests {
    //! Unit tests for merge helpers to ensure deterministic behaviour.
    use super::*;

    #[test]
    fn to_owned_vec_converts_all_entries() {
        let result = to_owned_vec(&["alpha", "beta"]);
        assert_eq!(result, vec![String::from("alpha"), String::from("beta")]);
    }

    #[test]
    fn environment_keys_deduplicates_and_appends_config_path() {
        let keys = environment_keys(&[
            ("ONE", Some("value")),
            ("VK_CONFIG_PATH", None),
            ("TWO", Some("x")),
        ]);
        assert_eq!(keys, vec!["ONE", "TWO", "VK_CONFIG_PATH"]);
    }
}

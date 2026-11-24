//! Shared helpers for merge configuration tests.
//!
//! Provides utilities for building owned strings from expected slices.

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

#[cfg(test)]
mod tests {
    //! Unit tests for merge helpers to ensure deterministic behaviour.
    use super::*;

    #[test]
    fn to_owned_vec_converts_all_entries() {
        let result = to_owned_vec(&["alpha", "beta"]);
        assert_eq!(result, vec![String::from("alpha"), String::from("beta")]);
    }
}

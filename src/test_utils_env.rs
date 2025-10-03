//! Test utilities used across integration and unit tests.
//!
//! This module provides helpers for managing environment variables during
//! tests and for normalising terminal output.

use crate::environment;

/// Remove ANSI escape sequences from a string.
///
/// # Examples
///
/// ```
/// use vk::test_utils::strip_ansi_codes;
/// let coloured = "\x1b[31mred\x1b[0m";
/// assert_eq!(strip_ansi_codes(coloured), "red");
/// ```
#[must_use]
pub fn strip_ansi_codes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && skip_ansi_sequence(&mut chars) {
            continue;
        }
        out.push(ch);
    }
    out
}

fn skip_ansi_sequence(chars: &mut impl Iterator<Item = char>) -> bool {
    if !matches!(chars.next(), Some('[')) {
        return false;
    }
    chars.any(|c| ('@'..='~').contains(&c))
}

/// Set an environment variable for testing.
///
/// Environment manipulation is process-wide and therefore not thread-safe.
/// A global mutex serialises modifications so parallel tests do not race.
pub fn set_var<K: AsRef<std::ffi::OsStr>, V: AsRef<std::ffi::OsStr>>(key: K, value: V) {
    environment::set_var(key, value);
}

/// Remove an environment variable set during testing.
///
/// The global mutex serialises modifications so parallel tests do not race.
pub fn remove_var<K: AsRef<std::ffi::OsStr>>(key: K) {
    environment::remove_var(key);
}

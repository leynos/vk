//! Test utilities used across integration and unit tests.
//!
//! This module provides helpers for managing environment variables during
//! tests and for normalising terminal output.

use crate::environment;

/// Assert that the provided text does not contain three consecutive newlines.
///
/// # Panics
///
/// Panics if `text` contains three consecutive newline characters.
/// # Examples
///
/// ```
/// use vk::test_utils::assert_no_triple_newlines;
///
/// assert_no_triple_newlines("a\n\nb");
/// ```
pub fn assert_no_triple_newlines(text: &str) {
    assert!(
        !text.contains("\n\n\n"),
        "output should not contain triple newlines:\n{text}"
    );
}

/// Assert that diff lines matching `pattern` are not separated by blank lines.
///
/// # Panics
///
/// Panics if fewer than three matching diff lines are present or if blank
/// lines appear between matches.
/// The helper expects unified diff output where the interesting lines begin
/// with either `-` or `+` followed by fourteen spaces and `pattern`.
///
/// # Examples
///
/// ```
/// use vk::test_utils::assert_diff_lines_contiguous;
///
/// let diff = "-              printf old\n+              printf new\n";
/// assert_diff_lines_contiguous(diff, "printf");
/// ```
pub fn assert_diff_lines_contiguous(text: &str, pattern: &str) {
    let lines: Vec<_> = text.lines().collect();
    let diff_line_numbers: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            let trimmed = line.trim_start();
            if trimmed.starts_with(&format!("-              {pattern}"))
                || trimmed.starts_with(&format!("+              {pattern}"))
            {
                Some(idx)
            } else {
                None
            }
        })
        .collect();

    assert!(
        diff_line_numbers.len() >= 3,
        "expected at least three diff lines containing '{pattern}':\n{text}"
    );

    for window in diff_line_numbers.windows(2) {
        let [first, second] = window else {
            continue;
        };
        let has_blank_separator = lines
            .get(first + 1..*second)
            .is_some_and(|slice| slice.iter().any(|line| line.trim().is_empty()));
        assert!(
            !has_blank_separator,
            "diff lines containing '{pattern}' should not be separated by blank lines:\n{text}"
        );
    }
}

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

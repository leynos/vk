//! Common utilities for manipulating environment variables in tests.

/// Set an environment variable for testing.
///
/// Environment manipulation is process-wide and therefore not thread-safe.
/// Callers must ensure tests using these helpers run serially, for example by
/// applying `#[serial]` from the `serial_test` crate to the test itself.
///
/// # Examples
///
/// ```ignore
/// use vk::test_utils::{set_var, remove_var};
///
/// set_var("MY_VAR", "1");
/// assert_eq!(std::env::var("MY_VAR"), Ok("1".into()));
/// remove_var("MY_VAR");
/// ```
pub fn set_var<K: AsRef<std::ffi::OsStr>, V: AsRef<std::ffi::OsStr>>(key: K, value: V) {
    // SAFETY: Tests using this helper run serially.
    unsafe { std::env::set_var(key, value) };
}

/// Remove an environment variable set during testing.
///
/// Callers must ensure tests using these helpers run serially.
pub fn remove_var<K: AsRef<std::ffi::OsStr>>(key: K) {
    // SAFETY: Tests using this helper run serially.
    unsafe { std::env::remove_var(key) };
}

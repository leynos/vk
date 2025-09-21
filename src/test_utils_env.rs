//! Test environment helpers.
//!
//! Provides functions for setting and removing environment variables in a
//! thread-safe manner for tests.

use crate::environment;

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

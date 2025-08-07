//! Common utilities for manipulating environment variables in tests.

/// Set an environment variable.
///
/// # Safety
///
/// Tests run serially so modifying process-wide state is safe here.
pub fn set_var<K: AsRef<std::ffi::OsStr>, V: AsRef<std::ffi::OsStr>>(key: K, value: V) {
    unsafe { std::env::set_var(key, value) }
}

/// Remove an environment variable.
///
/// # Safety
///
/// Tests run serially so modifying process-wide state is safe here.
pub fn remove_var<K: AsRef<std::ffi::OsStr>>(key: K) {
    unsafe { std::env::remove_var(key) }
}

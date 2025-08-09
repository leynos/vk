//! Common utilities for manipulating environment variables in tests.

use std::sync::{Mutex, OnceLock};

/// Set an environment variable for testing.
///
/// Environment manipulation is process-wide and therefore not thread-safe.
/// A global mutex serialises modifications so parallel tests do not race.
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
    let _guard = env_lock();
    // SAFETY: The global mutex serialises access to the environment.
    unsafe { std::env::set_var(key, value) };
}

/// Remove an environment variable set during testing.
///
/// The global mutex serialises modifications so parallel tests do not race.
pub fn remove_var<K: AsRef<std::ffi::OsStr>>(key: K) {
    let _guard = env_lock();
    // SAFETY: The global mutex serialises access to the environment.
    unsafe { std::env::remove_var(key) };
}

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock")
}

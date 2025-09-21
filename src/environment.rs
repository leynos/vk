//! Process-wide environment helpers.
//!
//! Provides synchronised wrappers around environment mutations so tests and
//! runtime code serialise access through a shared mutex.

use std::env;
use std::ffi::OsStr;
use std::sync::{Mutex, MutexGuard, OnceLock};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn lock() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("environment lock poisoned")
}

/// Set an environment variable while holding the global lock.
///
/// Environment variables are global to the process; without coordination these
/// operations are racy.
pub fn set_var<K: AsRef<OsStr>, V: AsRef<OsStr>>(key: K, value: V) {
    let _guard = lock();
    // SAFETY: the mutex serialises access to the unsynchronised std env calls.
    unsafe { env::set_var(key, value) };
}

/// Remove an environment variable while holding the global lock.
pub fn remove_var<K: AsRef<OsStr>>(key: K) {
    let _guard = lock();
    // SAFETY: the mutex serialises access to the unsynchronised std env calls.
    unsafe { env::remove_var(key) };
}

/// Read an environment variable while holding the global lock.
///
/// # Errors
///
/// Returns [`env::VarError`] when the variable is unset or contains invalid
/// Unicode.
pub fn var<K: AsRef<OsStr>>(key: K) -> Result<String, env::VarError> {
    let _guard = lock();
    env::var(key)
}

/// Run `op` while the environment mutex is held.
///
/// Tests occasionally need to snapshot multiple variables atomically; this
/// helper exposes the guard without leaking the concrete type.
pub fn with_lock<T, F>(op: F) -> T
where
    F: FnOnce() -> T,
{
    let _guard = lock();
    op()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;

    #[test]
    #[serial]
    fn set_var_round_trip() {
        let key = "VK_ENV_HELPER_TEST";
        let old = var(key).ok();
        set_var(key, "helper-value");
        assert_eq!(var(key).expect("read var"), "helper-value");
        match old {
            Some(value) => set_var(key, value),
            None => remove_var(key),
        }
    }

    #[test]
    #[serial]
    fn with_lock_allows_scoped_access() {
        let key = "VK_ENV_HELPER_LOCK_TEST";
        let (previous, snapshot) = with_lock(|| {
            let before = env::var(key).ok();
            // SAFETY: `with_lock` holds the guard for this closure.
            unsafe { env::set_var(key, "locked") };
            let after = env::var(key).ok();
            (before, after)
        });
        assert_eq!(snapshot.as_deref(), Some("locked"));
        match previous {
            Some(value) => set_var(key, value),
            None => remove_var(key),
        }
    }
}

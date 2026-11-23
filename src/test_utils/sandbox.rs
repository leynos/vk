//! Environment guards for tests.
//!
//! Provides the shared [`ENV_SANDBOX_LOCK`] mutex and the [`EnvGuard`] RAII
//! guard that coordinates process-wide environment variables between parallel
//! tests. Tests that touch these resources should additionally be marked
//! `#[serial]` to coordinate with helpers outwith this module.

use std::env;
use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard, OnceLock};
use vk::environment;

pub(super) static ENV_SANDBOX_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_sandbox_lock() -> MutexGuard<'static, ()> {
    ENV_SANDBOX_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("environment sandbox lock poisoned")
}

fn restore_env_key(key: &str, original: Option<OsString>) {
    environment::with_lock(|| match original {
        Some(value) => {
            // SAFETY: `environment::with_lock` serialises process-wide env access.
            #[expect(
                clippy::disallowed_methods,
                reason = "lock-guarded test helper; migrating to an injected environment"
            )]
            unsafe {
                env::set_var(key, value);
            }
        }
        None => {
            // SAFETY: `environment::with_lock` serialises process-wide env access.
            #[expect(
                clippy::disallowed_methods,
                reason = "lock-guarded test helper; migrating to an injected environment"
            )]
            unsafe {
                env::remove_var(key);
            }
        }
    });
}

/// Guard that restores an environment variable to its original value on drop.
pub struct EnvGuard {
    key: &'static str,
    original: Option<OsString>,
    _guard: MutexGuard<'static, ()>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        restore_env_key(self.key, self.original.take());
    }
}

/// Set `VK_HTTP_TIMEOUT` to an invalid value and restore it on drop.
#[must_use]
pub fn invalid_http_timeout_guard() -> EnvGuard {
    let guard = env_sandbox_lock();
    let original = environment::with_lock(|| {
        let original = env::var_os("VK_HTTP_TIMEOUT");
        // SAFETY: `EnvGuard` keeps `env_sandbox_lock()` held until drop.
        #[expect(
            clippy::disallowed_methods,
            reason = "lock-guarded test helper; migrating to an injected environment"
        )]
        unsafe {
            env::set_var("VK_HTTP_TIMEOUT", "not-a-number");
        }
        original
    });
    EnvGuard {
        key: "VK_HTTP_TIMEOUT",
        original,
        _guard: guard,
    }
}

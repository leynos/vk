//! Environment and working-directory sandboxing for tests.
//!
//! Provides the shared [`ENV_SANDBOX_LOCK`] mutex and the RAII guards
//! ([`EnvGuard`], [`EnvSandbox`], [`CwdGuard`]) that coordinate process-wide
//! mutable state — env vars and the current working directory — between
//! parallel tests. Every guard acquires the same mutex, and tests that touch
//! these resources should additionally be marked `#[serial]` to coordinate
//! with helpers outwith this module.

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use vk::environment;

pub(super) static ENV_SANDBOX_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

const SANDBOXED_ENV_KEYS: &[&str] = &[
    "VK_REPO",
    "VK_GITHUB_TOKEN",
    "VK_TRANSCRIPT",
    "VK_HTTP_TIMEOUT",
    "VK_CONNECT_TIMEOUT",
    "VK_CONFIG_PATH",
    "CONFIG_PATH",
    "APPDATA",
    "LOCALAPPDATA",
    "HOME",
    "XDG_CONFIG_HOME",
    "XDG_CONFIG_DIRS",
];

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
            unsafe { env::set_var(key, value) };
        }
        None => {
            // SAFETY: `environment::with_lock` serialises process-wide env access.
            unsafe { env::remove_var(key) };
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
        unsafe { env::set_var("VK_HTTP_TIMEOUT", "not-a-number") };
        original
    });
    EnvGuard {
        key: "VK_HTTP_TIMEOUT",
        original,
        _guard: guard,
    }
}

/// RAII guard that isolates configuration discovery inputs for a test.
///
/// The sandbox snapshots relevant configuration environment variables and the
/// current working directory, points discovery-related paths at an empty
/// temporary directory, and restores everything on drop.
///
/// # Examples
///
/// ```ignore
/// let sandbox = vk::test_utils::EnvSandbox::new().expect("create sandbox");
/// let config_path = sandbox.path().join("vk.toml");
/// ```
pub struct EnvSandbox {
    current_dir: PathBuf,
    original_env: Vec<(&'static str, Option<OsString>)>,
    sandbox_dir: tempfile::TempDir,
    _guard: MutexGuard<'static, ()>,
}

impl EnvSandbox {
    /// Create a new isolated environment and working-directory sandbox.
    ///
    /// # Errors
    ///
    /// Returns an error when the temporary directory or current working
    /// directory cannot be created or switched.
    pub fn new() -> io::Result<Self> {
        let guard = env_sandbox_lock();
        let sandbox_dir = tempfile::tempdir()?;
        let sandbox_path = sandbox_dir.path().to_path_buf();
        let current_dir = environment::with_lock(env::current_dir)?;
        let original_env = environment::with_lock(|| {
            SANDBOXED_ENV_KEYS
                .iter()
                .map(|key| (*key, env::var_os(key)))
                .collect::<Vec<_>>()
        });

        env::set_current_dir(&sandbox_path)?;
        environment::with_lock(|| {
            for key in SANDBOXED_ENV_KEYS {
                // SAFETY: `environment::with_lock` serialises process-wide env access.
                unsafe { env::remove_var(key) };
            }
            for key in [
                "APPDATA",
                "LOCALAPPDATA",
                "HOME",
                "XDG_CONFIG_HOME",
                "XDG_CONFIG_DIRS",
            ] {
                // SAFETY: `environment::with_lock` serialises process-wide env access.
                unsafe { env::set_var(key, &sandbox_path) };
            }
        });

        Ok(Self {
            current_dir,
            original_env,
            sandbox_dir,
            _guard: guard,
        })
    }

    /// Return the root path of the temporary discovery sandbox.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.sandbox_dir.path()
    }
}

impl Drop for EnvSandbox {
    fn drop(&mut self) {
        for (key, value) in self.original_env.drain(..) {
            restore_env_key(key, value);
        }
        let _ = env::set_current_dir(&self.current_dir);
    }
}

/// RAII guard that switches the process working directory and restores it on
/// drop, holding the global env/sandbox lock for its lifetime.
///
/// The current working directory is process-global, so concurrent tests that
/// each call `set_current_dir` race. `CwdGuard::enter` shares the same mutex
/// as [`EnvSandbox`], ensuring serialised access regardless of whether the
/// caller has remembered the `#[serial]` attribute — and tests still should
/// mark themselves `#[serial]` so they coordinate with other lock-aware
/// guards. The lock is held for the guard's entire lifetime, so the original
/// directory is restored under the same lock that switched it.
///
/// The lock is acquired with `try_lock` rather than a blocking `lock`. The
/// underlying mutex is **non-reentrant**, so a single test that already holds
/// it — for example, one that has constructed an [`EnvSandbox`] — would
/// otherwise deadlock here. With `try_lock` the same misuse fails fast with
/// [`io::ErrorKind::WouldBlock`], turning a hung test into a finite,
/// debuggable failure. Tests should still pick one guard per test rather than
/// relying on this fail-fast.
#[derive(Debug)]
pub struct CwdGuard {
    original: PathBuf,
    _guard: MutexGuard<'static, ()>,
}

impl CwdGuard {
    /// Switch the process cwd to `dir` for the lifetime of the guard.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::WouldBlock`] when the global env/sandbox lock
    /// is already held (typically because the test also constructed an
    /// [`EnvSandbox`] or another `CwdGuard`). Returns the underlying
    /// `io::Error` when the current directory cannot be read or when the
    /// chdir to `dir` fails.
    ///
    /// # Panics
    ///
    /// Panics if the env/sandbox mutex has been poisoned by a previous test
    /// panicking while holding it. Mirrors the behaviour of
    /// `env_sandbox_lock`.
    pub fn enter(dir: &Path) -> io::Result<Self> {
        let guard = match ENV_SANDBOX_LOCK.get_or_init(|| Mutex::new(())).try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(io::Error::from(io::ErrorKind::WouldBlock));
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                panic!("environment sandbox lock poisoned")
            }
        };
        let original = env::current_dir()?;
        env::set_current_dir(dir)?;
        Ok(Self {
            original,
            _guard: guard,
        })
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        // The lock is still held via `self._guard`, so the restore happens
        // atomically with respect to other lock-aware test helpers.
        let _ = env::set_current_dir(&self.original);
    }
}

#[cfg(test)]
mod cwd_guard_tests {
    use super::{CwdGuard, EnvSandbox};
    use serial_test::serial;
    use std::io;

    /// Pins the fail-fast contract: when the env/sandbox lock is already
    /// held, `CwdGuard::enter` must return `WouldBlock` immediately rather
    /// than block.
    #[test]
    #[serial]
    fn enter_reports_would_block_when_lock_held() {
        let sandbox = EnvSandbox::new().expect("create sandbox");
        let err =
            CwdGuard::enter(sandbox.path()).expect_err("CwdGuard must not acquire a held lock");
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        drop(sandbox);
    }
}

//! Environment and directory guards for integration tests.
//!
//! Provides helpers to capture and restore environment variables, temporarily
//! change the working directory, and write configuration files for CLI merge
//! tests.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use vk::test_utils::{remove_var, set_var};

/// RAII guard that restores captured environment variables on drop.
pub struct EnvGuard {
    entries: Vec<(OsString, Option<OsString>)>,
}

impl EnvGuard {
    /// Capture `keys`, removing them from the environment for the guard's
    /// lifetime.
    ///
    /// # Safety
    /// Mutating the process environment is globally visible. `EnvGuard` acquires
    /// the `vk::test_utils` environment lock so callers must serialise tests with
    /// `#[serial]` and include `VK_CONFIG_PATH` when configuration helpers are
    /// used.
    pub fn new(keys: &[&str]) -> Self {
        let mut entries = Vec::new();
        for key in keys {
            let key = OsString::from(key);
            let previous = env::var_os(&key);
            remove_var(&key);
            entries.push((key, previous));
        }
        Self { entries }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &mut self.entries {
            let key_ref = key.as_os_str();
            match value.take() {
                Some(val) => set_var(key_ref, val),
                None => remove_var(key_ref),
            }
        }
    }
}

/// RAII guard restoring the working directory on drop.
pub struct DirGuard {
    previous: PathBuf,
}

impl DirGuard {
    /// Enter `path`, returning a guard that restores the prior working
    /// directory when dropped.
    pub fn enter(path: impl AsRef<Path>) -> Self {
        let previous = env::current_dir().expect("current dir");
        env::set_current_dir(path.as_ref()).expect("set dir");
        Self { previous }
    }
}

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.previous);
    }
}

/// Write `content` to a temporary `.vk.toml` and return its directory and path.
pub fn write_config(content: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("create config dir");
    let path = dir.path().join(".vk.toml");
    fs::write(&path, content).expect("write config");
    (dir, path)
}

/// Write a config file, set `VK_CONFIG_PATH`, and return the directory and path.
///
/// Callers must create an [`EnvGuard`] that captures `VK_CONFIG_PATH` before
/// invoking this helper so the variable is removed once the guard drops.
pub fn setup_env_and_config(config_content: &str) -> (TempDir, PathBuf) {
    let (dir, path) = write_config(config_content);
    set_var("VK_CONFIG_PATH", path.as_os_str());
    (dir, path)
}

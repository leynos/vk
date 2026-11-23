//! Process environment reads.
//!
//! Production code reads the process environment through these helpers. Nothing
//! in the crate changes the process environment: tests inject an
//! `ortho_config::EnvSource` or run the binary as a child process instead.

use std::env;
use std::ffi::{OsStr, OsString};

/// Read an environment variable.
///
/// # Errors
///
/// Returns [`env::VarError`] when the variable is unset or contains invalid
/// Unicode.
pub fn var<K: AsRef<OsStr>>(key: K) -> Result<String, env::VarError> {
    env::var(key)
}

/// Read an environment variable as an `OsString`.
#[must_use]
pub fn var_os<K: AsRef<OsStr>>(key: K) -> Option<OsString> {
    env::var_os(key)
}

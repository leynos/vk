//! Common test utilities for the `vk` binary crate.
//!
//! Shared across the binary's unit tests and integration tests. The module is
//! intentionally split by concern so each file stays focused and below the
//! project's per-file size cap:
//!
//! - [`test_http`] — the [`TestClient`] HTTP stub server and [`start_server`]
//!   helper used to drive the GraphQL client without touching GitHub.
//! - [`sandbox`] — the [`EnvSandbox`] / [`EnvGuard`] / [`CwdGuard`] RAII
//!   guards plus the shared mutex they all hold to serialise mutations of
//!   process-global state (env vars and the current working directory).
//! - [`git_fixture`] — [`GitRepoFixture`], a hermetic temporary Git
//!   repository builder used by `git`-aware tests.
//!
//! A handful of small helpers from `vk::test_utils` (string assertions and
//! optional-env helpers) are re-exported here so callers can use the same
//! `crate::test_utils::…` path regardless of which file the helper lives in.

mod git_fixture;
mod sandbox;
mod test_http;

pub use git_fixture::GitRepoFixture;
pub use sandbox::{CwdGuard, EnvGuard, EnvSandbox, invalid_http_timeout_guard};
pub use test_http::{TestClient, start_server};
pub use vk::test_utils::{
    apply_optional_env, assert_diff_lines_not_blank_separated, assert_no_triple_newlines,
    restore_optional_env, strip_ansi_codes,
};

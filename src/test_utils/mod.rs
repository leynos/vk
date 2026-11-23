//! Common test utilities for the `vk` binary crate.
//!
//! Shared across the binary's unit tests and integration tests. The module is
//! intentionally split by concern so each file stays focused and below the
//! project's per-file size cap:
//!
//! - [`test_http`] — the [`TestClient`] HTTP stub server and [`start_server`]
//!   helper used to drive the GraphQL client without touching GitHub.
//! - [`git_fixture`] — [`GitRepoFixture`], a hermetic temporary Git
//!   repository builder used by `git`-aware tests.
//!
//! A handful of small helpers from `vk::test_utils` (string assertions) are re-exported here so callers can use the same
//! `crate::test_utils::…` path regardless of which file the helper lives in.

mod git_fixture;
mod test_http;

pub use git_fixture::GitRepoFixture;
pub use test_http::{TestClient, start_server};
pub use vk::test_utils::{
    assert_diff_lines_not_blank_separated, assert_no_triple_newlines, strip_ansi_codes,
};

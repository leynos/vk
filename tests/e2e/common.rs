//! Shared end-to-end test utilities: fixtures, helpers, and mock infrastructure.
//!
//! This module provides common building blocks for e2e tests, including git
//! repository fixtures and helper functions for setting up test scenarios.
//!
//! Each test spawns a [`third-wheel`](https://crates.io/crates/third-wheel)
//! Man-in-the-Middle proxy that intercepts outbound GitHub requests. This
//! proxy serves canned responses from `tests/fixtures` so the suite runs in a
//! fully hermetic and deterministic manner.

use rstest::fixture;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::{TempDir, tempdir};

#[path = "../utils/mod.rs"]
pub mod utils;

pub use utils::{
    set_sequential_responder, set_sequential_responder_with_assert, start_mitm, start_mitm_capture,
    vk_cmd,
};

/// Initialize a git repository in the given directory and set HEAD appropriately.
///
/// Uses `git -c init.defaultBranch=main init` for compatibility with Git
/// versions older than 2.28 which don't support `--initial-branch`.
///
/// The `head_content` parameter can be:
/// - `"ref: refs/heads/<branch>\n"` to set HEAD to a branch
/// - A commit SHA (anything else) to create a detached HEAD state
pub fn init_git_repo(dir: &std::path::Path, head_content: &str) {
    use std::process::Command as StdCommand;

    // Initialize a real git repository so git rev-parse works
    // Use -c init.defaultBranch=main for compatibility with Git < 2.28
    let status = StdCommand::new("git")
        .args(["-c", "init.defaultBranch=main", "init"])
        .current_dir(dir)
        .output()
        .expect("git init");
    assert!(status.status.success(), "git init failed");

    // Check if head_content is a symbolic ref or a detached state
    let trimmed = head_content.trim();
    if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
        // Use git symbolic-ref to set HEAD to the desired branch
        let status = StdCommand::new("git")
            .args(["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")])
            .current_dir(dir)
            .output()
            .expect("git symbolic-ref");
        assert!(status.status.success(), "git symbolic-ref failed");
    } else {
        // For detached HEAD, we need a commit to detach to
        // Configure user for commit
        let status = StdCommand::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .expect("git config email");
        assert!(status.status.success(), "git config email failed");
        let status = StdCommand::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .expect("git config name");
        assert!(status.status.success(), "git config name failed");

        // Create an empty commit
        let status = StdCommand::new("git")
            .args(["commit", "--allow-empty", "-m", "initial"])
            .current_dir(dir)
            .output()
            .expect("git commit");
        assert!(status.status.success(), "git commit failed");

        // Detach HEAD
        let status = StdCommand::new("git")
            .args(["checkout", "--detach"])
            .current_dir(dir)
            .output()
            .expect("git checkout --detach");
        assert!(status.status.success(), "git checkout --detach failed");
    }
}

/// A temporary Git repository with configurable `HEAD` and `FETCH_HEAD`.
///
/// This struct encapsulates the common setup pattern of creating a temp
/// directory, initializing git, and writing `HEAD`/`FETCH_HEAD` files.
pub struct GitRepoWithFetchHead {
    dir: TempDir,
}

/// Default `FETCH_HEAD` content pointing to a GitHub repo.
pub const DEFAULT_FETCH_HEAD: &str =
    "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/owner/repo.git";

impl GitRepoWithFetchHead {
    /// Create a new git repository fixture with the given `HEAD` and `FETCH_HEAD` content.
    pub fn new(head_content: &str, fetch_head_content: &str) -> Self {
        let dir = tempdir().expect("tempdir");
        init_git_repo(dir.path(), head_content);
        let git_dir = dir.path().join(".git");
        fs::write(git_dir.join("FETCH_HEAD"), fetch_head_content).expect("write FETCH_HEAD");
        Self { dir }
    }

    /// Create a new git repository fixture with custom `HEAD` and default `FETCH_HEAD`.
    pub fn with_head(head_content: &str) -> Self {
        Self::new(head_content, DEFAULT_FETCH_HEAD)
    }

    /// Get the path to the temporary directory.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// rstest fixture for creating a git repository with configurable HEAD and `FETCH_HEAD`.
///
/// Default HEAD is `ref: refs/heads/main\n` (on branch main).
/// Default `FETCH_HEAD` points to `https://github.com/owner/repo.git`.
#[fixture]
pub fn git_repo_with_fetch_head(
    #[default("ref: refs/heads/main\n")] head: &str,
    #[default(DEFAULT_FETCH_HEAD)] fetch_head: &str,
) -> GitRepoWithFetchHead {
    GitRepoWithFetchHead::new(head, fetch_head)
}

pub fn load_transcript(path: &str) -> Vec<String> {
    let data = fs::read_to_string(path).expect("read transcript");
    data.lines()
        .map(|line| {
            let v: Value = serde_json::from_str(line).expect("valid json line");
            v.get("response")
                .and_then(|r| r.as_str())
                .unwrap_or("{}")
                .to_owned()
        })
        .collect()
}

/// Build a default empty `comments` payload.
pub fn empty_comments_fallback() -> String {
    serde_json::json!({
        "data": {"node": {"comments": {
            "nodes": [],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}
    })
    .to_string()
}

/// Build response bodies for fork disambiguation tests.
///
/// Takes a slice of (PR number, owner name) tuples and returns:
/// - `pr_lookup_body`: JSON with multiple PRs from different forks
/// - `threads_body`: Empty threads response
/// - `reviews_body`: Empty reviews response loaded from fixture
pub fn fork_disambiguation_responses(fork_prs: &[(u64, &str)]) -> (String, String, String) {
    let nodes: Vec<serde_json::Value> = fork_prs
        .iter()
        .map(|(number, owner)| {
            serde_json::json!({
                "number": number,
                "headRepository": {
                    "owner": {"login": owner}
                }
            })
        })
        .collect();

    let pr_lookup_body = serde_json::json!({
        "data": {"repository": {"pullRequests": {
            "nodes": nodes
        }}}
    })
    .to_string();

    let threads_body = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}}
    })
    .to_string();

    let reviews_body = include_str!("../fixtures/reviews_empty.json").to_string();

    (pr_lookup_body, threads_body, reviews_body)
}

/// Add an origin remote to a git repository.
pub fn add_origin_remote(repo_path: &Path, origin_url: &str) {
    use std::process::Command as StdCommand;

    let status = StdCommand::new("git")
        .args(["remote", "add", "origin", origin_url])
        .current_dir(repo_path)
        .output()
        .expect("git remote add");
    assert!(status.status.success(), "git remote add failed");
}

/// Create a request asserter that verifies a specific PR number is selected.
///
/// Returns a closure that tracks request count and asserts the expected PR number
/// appears in the threads query (second request). This is used to verify fork
/// disambiguation selects the correct PR.
pub fn assert_pr_number_on_threads_query(
    expected_pr: u64,
) -> (
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    impl Fn(&serde_json::Value) + Send + Sync + 'static,
) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let request_count = Arc::new(AtomicUsize::new(0));
    let request_count_clone = Arc::clone(&request_count);

    let asserter = move |body: &serde_json::Value| {
        let count = request_count_clone.fetch_add(1, Ordering::SeqCst);
        // Assert on the second request (threads query) to verify correct PR was selected
        if count == 1 {
            let vars = &body["variables"];
            assert_eq!(
                vars["number"], expected_pr,
                "Should select PR #{expected_pr}"
            );
        }
    };

    (request_count, asserter)
}

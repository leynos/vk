//! Tests for PR auto-detection from current branch and fork disambiguation.

use super::common::*;
use assert_cmd::prelude::*;
use predicates::str::contains;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[tokio::test]
async fn pr_auto_detects_from_branch() {
    let (addr, handler, shutdown) = start_mitm_capture().await.expect("start server");

    // PR lookup now includes headRepository for fork disambiguation
    let pr_lookup_body = serde_json::json!({
        "data": {"repository": {"pullRequests": {
            "nodes": [{
                "number": 42,
                "headRepository": {
                    "owner": {"login": "owner"}
                }
            }]
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

    // Track which request we're on to only assert on the first (PR lookup) request
    let request_count = Arc::new(AtomicUsize::new(0));
    let request_count_clone = Arc::clone(&request_count);

    set_sequential_responder_with_assert(
        &handler,
        vec![pr_lookup_body, threads_body, reviews_body],
        move |body: &serde_json::Value| {
            let count = request_count_clone.fetch_add(1, Ordering::SeqCst);
            // Only assert on the first request (PR lookup by branch)
            if count == 0 {
                let vars = &body["variables"];
                assert_eq!(
                    vars["headRef"], "my-feature-branch",
                    "GraphQL headRef should match branch from .git/HEAD"
                );
                assert_eq!(
                    vars["owner"], "owner",
                    "GraphQL owner should match repo from FETCH_HEAD"
                );
                assert_eq!(
                    vars["name"], "repo",
                    "GraphQL name should match repo from FETCH_HEAD"
                );
            }
        },
    );

    let repo = GitRepoWithFetchHead::new(
        "ref: refs/heads/my-feature-branch\n",
        "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/owner/repo.git",
    );

    tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            vk_cmd(addr)
                .current_dir(repo.path())
                .args(["pr"])
                .assert()
                .success()
                .stdout(contains("No unresolved comments"));
        }),
    )
    .await
    .expect("command timed out")
    .expect("spawn blocking");
    shutdown.shutdown().await;
}

#[tokio::test]
async fn pr_fragment_only_auto_detects_pr() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");

    let pr_lookup_body = serde_json::json!({
        "data": {"repository": {"pullRequests": {
            "nodes": [{
                "number": 7,
                "headRepository": {
                    "owner": {"login": "o"}
                }
            }]
        }}}
    })
    .to_string();
    let threads_body = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [{
                "id": "t1",
                "isResolved": false,
                "isOutdated": false,
                "comments": {
                    "nodes": [{
                        "body": "fragment comment",
                        "diffHunk": "@@ -1 +1 @@\n-old\n+new\n",
                        "originalPosition": null,
                        "position": null,
                        "path": "file.rs",
                        "url": "https://github.com/o/r/pull/7#discussion_r99",
                        "author": null
                    }],
                    "pageInfo": {"hasNextPage": false, "endCursor": null}
                }
            }],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}}
    })
    .to_string();
    let reviews_body = include_str!("../fixtures/reviews_empty.json").to_string();
    set_sequential_responder(&handler, vec![pr_lookup_body, threads_body, reviews_body]);

    let repo = GitRepoWithFetchHead::new(
        "ref: refs/heads/feature\n",
        "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/o/r.git",
    );

    tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            vk_cmd(addr)
                .current_dir(repo.path())
                .args(["pr", "#discussion_r99"])
                .assert()
                .success()
                .stdout(contains("fragment comment"));
        }),
    )
    .await
    .expect("command timed out")
    .expect("spawn blocking");
    shutdown.shutdown().await;
}

#[tokio::test]
async fn pr_no_reference_fails_on_detached_head() {
    let (addr, _handler, shutdown) = start_mitm().await.expect("start server");

    // Initialize git but set HEAD to a detached state (commit SHA)
    let repo = GitRepoWithFetchHead::with_head("abc123def456\n");

    tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            vk_cmd(addr)
                .current_dir(repo.path())
                .args(["pr"])
                .assert()
                .failure()
                .stderr(contains("detached HEAD state"));
        }),
    )
    .await
    .expect("command timed out")
    .expect("spawn blocking");
    shutdown.shutdown().await;
}

#[tokio::test]
async fn pr_no_reference_fails_when_no_pr_for_branch() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");

    let pr_lookup_body = serde_json::json!({
        "data": {"repository": {"pullRequests": {
            "nodes": []
        }}}
    })
    .to_string();
    set_sequential_responder(&handler, vec![pr_lookup_body]);

    let repo = GitRepoWithFetchHead::with_head("ref: refs/heads/orphan-branch\n");

    tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            vk_cmd(addr)
                .current_dir(repo.path())
                .args(["pr"])
                .assert()
                .failure()
                .stderr(contains("no pull request found for branch 'orphan-branch'"));
        }),
    )
    .await
    .expect("command timed out")
    .expect("spawn blocking");
    shutdown.shutdown().await;
}

#[tokio::test]
async fn pr_fork_disambiguation_selects_correct_pr() {
    let (addr, handler, shutdown) = start_mitm_capture().await.expect("start server");

    // Multiple PRs with the same branch name from different forks.
    // The user's fork is "my-fork", and there's also PRs from "other-fork" and
    // "another-fork" with the same branch name.
    let (pr_lookup_body, threads_body, reviews_body) = fork_disambiguation_responses(&[
        (100, "other-fork"),
        (200, "my-fork"),
        (300, "another-fork"),
    ]);

    // Track request count to assert on the second request (threads query)
    let request_count = Arc::new(AtomicUsize::new(0));
    let request_count_clone = Arc::clone(&request_count);

    set_sequential_responder_with_assert(
        &handler,
        vec![pr_lookup_body, threads_body, reviews_body],
        move |body: &serde_json::Value| {
            let count = request_count_clone.fetch_add(1, Ordering::SeqCst);
            // Assert on the second request (threads query) to verify PR #200 was selected
            if count == 1 {
                let vars = &body["variables"];
                assert_eq!(
                    vars["number"], 200,
                    "Should select PR #200 from my-fork, not #100 or #300"
                );
            }
        },
    );

    // Create a repo with origin pointing to my-fork
    let repo = GitRepoWithFetchHead::new(
        "ref: refs/heads/feature-branch\n",
        "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/upstream/repo.git",
    );
    add_origin_remote(repo.path(), "https://github.com/my-fork/repo.git");

    tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            // Use --repo to specify the upstream repository
            vk_cmd(addr)
                .current_dir(repo.path())
                .args(["--repo", "upstream/repo", "pr"])
                .assert()
                .success()
                .stdout(contains("No unresolved comments"));
        }),
    )
    .await
    .expect("command timed out")
    .expect("spawn blocking");
    shutdown.shutdown().await;
}

#[tokio::test]
async fn pr_fork_disambiguation_falls_back_to_first_when_no_origin() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");

    // Multiple PRs with the same branch name - without origin remote configured,
    // it should fall back to the first PR.
    let pr_lookup_body = serde_json::json!({
        "data": {"repository": {"pullRequests": {
            "nodes": [
                {
                    "number": 100,
                    "headRepository": {
                        "owner": {"login": "first-fork"}
                    }
                },
                {
                    "number": 200,
                    "headRepository": {
                        "owner": {"login": "second-fork"}
                    }
                }
            ]
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
    set_sequential_responder(&handler, vec![pr_lookup_body, threads_body, reviews_body]);

    // Create a repo WITHOUT origin remote - only FETCH_HEAD
    let repo = GitRepoWithFetchHead::new(
        "ref: refs/heads/feature-branch\n",
        "deadbeef\tnot-for-merge\tbranch 'main' of https://github.com/upstream/repo.git",
    );

    tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || {
            // Without origin remote, should fall back to first PR (#100)
            vk_cmd(addr)
                .current_dir(repo.path())
                .args(["pr"])
                .assert()
                .success()
                .stdout(contains("No unresolved comments"));
        }),
    )
    .await
    .expect("command timed out")
    .expect("spawn blocking");
    shutdown.shutdown().await;
}

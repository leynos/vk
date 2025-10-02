//! CLI integration tests for `vk pr` output.
//!
//! These tests verify empty-state messaging and the final summary generation
//! across multiple files.

use assert_cmd::prelude::*;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode, body::Incoming};
use insta::assert_snapshot;
use rstest::rstest;
use serde_json::json;
use vk::banners::{COMMENTS_BANNER, END_BANNER, START_BANNER};

mod utils;
use utils::{start_mitm, vk_cmd};

/// Build a closure returning an empty `reviewThreads` payload.
fn create_empty_review_handler()
-> impl Fn(&Request<Incoming>) -> Response<Full<Bytes>> + Send + 'static {
    move |_req| {
        let body = json!({
            "data": {
                "repository": {
                    "pullRequest": {
                        "reviewThreads": {
                            "nodes": [],
                            "pageInfo": { "hasNextPage": false, "endCursor": null }
                        },
                        "reviews": {
                            "nodes": [],
                            "pageInfo": { "hasNextPage": false, "endCursor": null }
                        }
                    }
                }
            }
        })
        .to_string();
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("build response")
    }
}

fn strip_ansi_codes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == (0x1b as char) {
            if chars.next().is_some_and(|next| next == '[') {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[rstest]
#[case(
    Vec::new(),
    format!("{START_BANNER}\nNo unresolved comments.\n{END_BANNER}\n"),
)]
#[case(
    vec!["no_such_file.rs"],
    format!(
        "{START_BANNER}\nNo unresolved comments for the specified files.\n{END_BANNER}\n"
    ),
)]
#[tokio::test]
async fn pr_empty_state(#[case] extra_args: Vec<&'static str>, #[case] expected_output: String) {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    *handler.lock().expect("lock handler") = Box::new(create_empty_review_handler());

    let output = expected_output;

    tokio::task::spawn_blocking(move || {
        let mut cmd = vk_cmd(addr);
        cmd.args(["pr", "https://github.com/leynos/shared-actions/pull/42"]);
        for arg in extra_args {
            cmd.arg(arg);
        }
        cmd.assert().success().stdout(output);
    })
    .await
    .expect("spawn blocking");

    shutdown.shutdown().await;
}

#[tokio::test]
async fn pr_outputs_banner_when_threads_present() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let threads_body = json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [{
                "id": "t1",
                "isResolved": false,
                "isOutdated": false,
                "comments": {
                    "nodes": [{
                        "body": "Looks good",
                        "diffHunk": "@@ -1 +1 @@\n-old\n+new\n",
                        "originalPosition": null,
                        "position": null,
                        "path": "file.rs",
                        "url": "http://example.com",
                        "author": {"login": "alice"}
                    }],
                    "pageInfo": {"hasNextPage": false, "endCursor": null}
                }
            }],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}}
    })
    .to_string();
    let reviews_body = json!({
        "data": {"repository": {"pullRequest": {"reviews": {
            "nodes": [],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}}
    })
    .to_string();
    let mut responses = vec![threads_body, reviews_body].into_iter();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = responses.next().expect("response");
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("build response")
    });

    tokio::task::spawn_blocking(move || {
        let mut cmd = vk_cmd(addr);
        cmd.args(["pr", "https://github.com/leynos/shared-actions/pull/42"]);
        let output = cmd.assert().success().get_output().stdout.clone();
        let output_str = String::from_utf8_lossy(&output);
        validate_banner_content(&output_str);
        validate_banner_ordering(&output_str);
    })
    .await
    .expect("spawn blocking");

    shutdown.shutdown().await;
}

/// Confirm banners and comment text are present in the output.
fn validate_banner_content(output: &str) {
    assert!(
        output.starts_with(&format!("{START_BANNER}\n")),
        "Output should start with code review banner",
    );
    assert!(
        output.contains(COMMENTS_BANNER),
        "Output should contain review comments banner",
    );
    let occurrences = output.match_indices(COMMENTS_BANNER).count();
    assert_eq!(
        occurrences, 1,
        "Review comments banner should appear exactly once",
    );
    assert!(
        output.contains("Looks good"),
        "Output should contain 'Looks good'",
    );
    assert!(
        output.contains(END_BANNER),
        "Output should contain end banner",
    );
}

/// Ensure banners and thread output appear in the expected order.
fn validate_banner_ordering(output: &str) {
    let code_idx = output.find(START_BANNER).expect("code review banner");
    let review_idx = output
        .find(COMMENTS_BANNER)
        .expect("review comments banner");
    let thread_idx = output.find("Looks good").expect("thread output");
    let end_idx = output.rfind(END_BANNER).expect("end banner");
    assert!(
        code_idx < review_idx,
        "Review comments banner should appear after code review banner",
    );
    assert!(
        review_idx < thread_idx,
        "Thread output should appear after review comments banner",
    );
    assert!(
        thread_idx < end_idx,
        "End banner should appear after thread output",
    );
}

#[tokio::test]
async fn pr_summarises_multiple_files() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let threads_body = include_str!("fixtures/review_threads_multiple_files.json").to_string();
    let reviews_body = include_str!("fixtures/reviews_empty.json").to_string();
    let last = reviews_body.clone();
    let mut responses = vec![threads_body.clone(), reviews_body].into_iter();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = responses.next().unwrap_or_else(|| last.clone());
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("build response")
    });

    let stdout = tokio::task::spawn_blocking(move || {
        let mut cmd = vk_cmd(addr);
        cmd.args(["pr", "https://github.com/leynos/shared-actions/pull/42"]);
        let output = cmd.output().expect("run command");
        assert!(
            output.status.success(),
            "vk exited with {:?}. Stderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf8")
    })
    .await
    .expect("spawn blocking");

    let stdout = stdout.replace("\r\n", "\n");
    let body = stdout
        .split_once("Summary:\n")
        .map_or(stdout.as_str(), |(_, rest)| {
            rest.split("\n\n").next().unwrap_or(rest)
        });
    let summary = format!("Summary:\n{body}");
    assert_snapshot!("pr_summarises_multiple_files_summary", summary);

    shutdown.shutdown().await;
}

#[tokio::test]
async fn pr_renders_coderabbit_comment_without_extra_spacing() {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let threads_body = include_str!("fixtures/review_threads_coderabbit.json").to_string();
    let reviews_body = include_str!("fixtures/reviews_empty.json").to_string();
    let last = reviews_body.clone();
    let mut responses = vec![threads_body, reviews_body].into_iter();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = responses.next().unwrap_or_else(|| last.clone());
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from(body))
            .expect("build response")
    });

    let stdout = tokio::task::spawn_blocking(move || {
        let mut cmd = vk_cmd(addr);
        cmd.args(["pr", "https://github.com/leynos/netsuke/pull/177"]);
        let output = cmd.output().expect("run command");
        assert!(
            output.status.success(),
            "vk exited with {:?}. Stderr:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf8")
    })
    .await
    .expect("spawn blocking");

    let stdout = stdout.replace("\r\n", "\n");
    let lines: Vec<_> = stdout.lines().collect();
    let start = lines
        .iter()
        .position(|line| strip_ansi_codes(line).contains("coderabbitai wrote"))
        .expect("comment start");
    let tail = lines.get(start..).unwrap_or(&[]);
    let end = tail
        .iter()
        .position(|line| line.starts_with("https://"))
        .map_or(lines.len(), |idx| start + idx);
    let comment_section = lines
        .get(start..end)
        .map_or_else(String::new, |slice| slice.join("\n"));

    let plain = strip_ansi_codes(&comment_section);
    assert!(
        !plain.contains("\n\n\n"),
        "comment should not contain triple newlines:\n{plain}"
    );

    let diff_line_numbers: Vec<_> = plain
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("-              printf")
                || trimmed.starts_with("+              printf")
            {
                Some(idx)
            } else {
                None
            }
        })
        .collect();
    assert!(
        diff_line_numbers.len() == 3,
        "expected three diff lines:\n{plain}"
    );
    for window in diff_line_numbers.windows(2) {
        let [first, second] = window else {
            continue;
        };
        assert_eq!(
            first + 1,
            *second,
            "diff lines should be contiguous:\n{plain}"
        );
    }

    assert_snapshot!("pr_renders_coderabbit_comment", plain);

    shutdown.shutdown().await;
}

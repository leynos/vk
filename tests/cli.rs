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
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use vk::banners::{COMMENTS_BANNER, END_BANNER, START_BANNER};
use vk::icons::{ICON_COMMENT, ICON_FILE, ICON_PERMALINK};
use vk::test_utils::{
    assert_diff_lines_not_blank_separated, assert_no_triple_newlines, strip_ansi_codes,
};

mod utils;
use utils::{ShutdownHandle as MitmShutdown, start_mitm, vk_cmd};

type RequestHandler = Box<dyn FnMut(&Request<Incoming>) -> Response<Full<Bytes>> + Send>;

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
    let (addr, _handler, shutdown) = setup_mock_server_for_coderabbit_test().await;

    let stdout = run_cli_and_capture_output(addr).await;
    let plain = extract_coderabbit_comment_section(&stdout);

    assert_no_triple_newlines(&plain);
    assert_diff_lines_not_blank_separated(&plain, "printf");

    assert_snapshot!("pr_renders_coderabbit_comment", plain);
    shutdown.shutdown().await;
}

async fn setup_mock_server_for_coderabbit_test()
-> (SocketAddr, Arc<Mutex<RequestHandler>>, MitmShutdown) {
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let handler: Arc<Mutex<RequestHandler>> = handler;
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
    (addr, handler, shutdown)
}

async fn run_cli_and_capture_output(addr: SocketAddr) -> String {
    tokio::task::spawn_blocking(move || {
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
    .expect("spawn blocking")
}

/// Compose a `write_thread`-shaped comment block for use in extractor tests.
fn fake_thread_block(url: &str, path: &str, author: &str, body: &str) -> String {
    format!(
        "\n{ICON_PERMALINK} {url}\n\n{ICON_FILE} {path}:\n    1|-old\n    1|+new\n\n\
         {ICON_COMMENT}  {author} wrote:\n{body}\n\n---\n",
    )
}

#[test]
fn extract_coderabbit_comment_section_skips_non_coderabbit_threads() {
    let stdout = format!(
        "{}{}{}",
        fake_thread_block(
            "https://example.com#discussion_r1",
            "a.rs",
            "alice",
            "alice body",
        ),
        fake_thread_block(
            "https://example.com#discussion_r2",
            "b.rs",
            "coderabbitai",
            "rabbit body",
        ),
        fake_thread_block(
            "https://example.com#discussion_r3",
            "c.rs",
            "bob",
            "bob body",
        ),
    );

    let extracted = extract_coderabbit_comment_section(&stdout);

    assert!(
        extracted.starts_with(&format!(
            "{ICON_PERMALINK} https://example.com#discussion_r2"
        )),
        "extracted block must start at the coderabbit permalink: {extracted}"
    );
    assert!(extracted.ends_with("---"));
    assert!(extracted.contains("coderabbitai wrote:"));
    assert!(extracted.contains("rabbit body"));
    // The extractor must not bleed into the alice or bob sections.
    assert!(!extracted.contains("alice"));
    assert!(!extracted.contains("alice body"));
    assert!(!extracted.contains("bob"));
    assert!(!extracted.contains("bob body"));
    assert!(!extracted.contains("discussion_r1"));
    assert!(!extracted.contains("discussion_r3"));
}

fn extract_coderabbit_comment_section(stdout: &str) -> String {
    let stdout = stdout.replace("\r\n", "\n");
    let stdout = strip_ansi_codes(&stdout);
    let lines: Vec<_> = stdout.lines().collect();
    let url_prefix = format!("{ICON_PERMALINK} ");
    let coderabbit_marker = "coderabbitai wrote:";
    // Locate the coderabbit author banner first, then walk backwards to the
    // permalink line that opens its comment block. This binds the extracted
    // section to the coderabbit thread even if the fixture grows to contain
    // additional threads.
    let marker_idx = lines
        .iter()
        .position(|line| line.contains(coderabbit_marker))
        .expect("coderabbit author banner");
    let start = lines
        .get(..marker_idx)
        .unwrap_or(&[])
        .iter()
        .rposition(|line| line.starts_with(&url_prefix))
        .expect("coderabbit permalink line preceding banner");
    let tail = lines.get(start..).unwrap_or(&[]);
    let end_offset = tail
        .iter()
        .position(|line| *line == "---")
        .map_or(tail.len(), |idx| idx + 1);
    let end = start + end_offset;
    lines
        .get(start..end)
        .map_or_else(String::new, |slice| slice.join("\n"))
}

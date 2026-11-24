//! Process-level precedence checks for the production `load_and_merge` path.
//!
//! The in-process merge suites inject an environment. These tests run the real
//! `vk` binary, whose `main` merges subcommand configuration through the
//! process environment, and observe precedence through the one signal that
//! needs no GitHub access: a file value of the wrong type fails the merge only
//! when no higher-precedence source replaces it. When the merge succeeds the
//! binary reaches a local stub server, which proves it got past the merge.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use rstest::rstest;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

#[path = "support/capfs.rs"]
mod capfs;

/// Local HTTP stub that records the request line of every connection it accepts.
struct StubServer {
    url: String,
    request_lines: Arc<Mutex<Vec<String>>>,
}

impl StubServer {
    /// Bind an ephemeral loopback port and answer every request with a 500.
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the stub server");
        let url = format!("http://{}", listener.local_addr().expect("stub address"));
        let request_lines = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&request_lines);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream);
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                recorded
                    .lock()
                    .expect("stub request log")
                    .push(first.trim_end().to_owned());
                // Drain the headers so the client sees a well-formed exchange.
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
                    line.clear();
                }
                // The client may already have gone; there is nobody to tell.
                drop(reader.get_mut().write_all(
                    b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                ));
            }
        });
        Self { url, request_lines }
    }

    /// Request lines received so far.
    fn requests(&self) -> Vec<String> {
        self.request_lines.lock().expect("stub request log").clone()
    }
}

/// Run `vk pr` against a config file whose `show_outdated` has the wrong type.
///
/// HOME and the XDG locations name the config directory so the host's own
/// configuration cannot take part, and the API endpoint is `stub`.
fn run_with_bad_file_value(
    stub: &StubServer,
    show_outdated_env: Option<&str>,
) -> assert_cmd::assert::Assert {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join(".vk.toml");
    capfs::write_file(&path, "[cmds.pr]\nshow_outdated = \"later\"\n").expect("write config");

    let mut cmd = Command::cargo_bin("vk").expect("binary");
    cmd.env_clear()
        .env("VK_CONFIG_PATH", &path)
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path())
        .env("XDG_CONFIG_DIRS", dir.path())
        .env("GITHUB_GRAPHQL_URL", &stub.url)
        .current_dir(dir.path())
        .args(["--repo", "foo/bar", "pr", "1"]);
    if let Some(value) = show_outdated_env {
        cmd.env("VK_CMDS_PR_SHOW_OUTDATED", value);
    }
    cmd.assert()
}

#[rstest]
fn file_value_of_the_wrong_type_fails_the_merge_without_an_environment_override() {
    let stub = StubServer::start();

    run_with_bad_file_value(&stub, None)
        .failure()
        .stderr(predicate::str::contains("configuration error"));

    assert!(
        stub.requests().is_empty(),
        "a failed merge must not reach the API: {:?}",
        stub.requests()
    );
}

#[rstest]
fn environment_value_replaces_the_file_value_before_it_is_deserialized() {
    let stub = StubServer::start();

    run_with_bad_file_value(&stub, Some("true"))
        .failure()
        .stderr(predicate::str::contains("configuration error").not());

    let requests = stub.requests();
    assert!(
        !requests.is_empty() && requests.iter().all(|line| line.starts_with("POST ")),
        "the merged configuration should reach the API with GraphQL POST requests: {requests:?}"
    );
}

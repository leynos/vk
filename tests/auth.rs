//! Authentication tests for GitHub token sources.
//!
//! Verifies that vk includes the `Authorization` header when a token is
//! supplied via configuration files, `VK_GITHUB_TOKEN`, `GITHUB_TOKEN`, or
//! CLI arguments, and warns when no token is available.

use assert_cmd::prelude::*;
use http_body_util::Full;
use hyper::{Request, Response, StatusCode, body::Incoming, header};
use rstest::rstest;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tokio::time::{Duration, timeout};

mod utils;
use utils::start_mitm;

/// GraphQL body with empty threads and reviews.
const EMPTY_REVIEW_BODY: &str = r#"{
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
}"#;

const ISSUE_BODY: &str = r#"{
  "data": {
    "repository": {
      "issue": {
        "title": "Title",
        "body": "Body"
      }
    }
  }
}"#;

const ANON_WARNING: &str = "GitHub token not set, using anonymous API access";

#[derive(Clone, Copy, Debug)]
enum VkCommand {
    Pr,
    Issue,
}

#[derive(Clone, Copy, Debug)]
enum TokenSource {
    Config,
    VkEnv,
    Cli,
}

impl TokenSource {
    fn config_contents(self) -> &'static str {
        match self {
            Self::Config => "github_token = \"dummy\"",
            _ => "",
        }
    }

    fn configure(self, cmd: &mut Command) {
        cmd.env_remove("GITHUB_TOKEN").env_remove("VK_GITHUB_TOKEN");
        match self {
            Self::Config => {}
            Self::VkEnv => {
                cmd.env("VK_GITHUB_TOKEN", "dummy");
            }
            Self::Cli => {
                cmd.env("VK_GITHUB_TOKEN", "env-token")
                    .args(["--github-token", "dummy"]);
            }
        }
    }
}

impl VkCommand {
    fn args(self) -> &'static [&'static str] {
        match self {
            Self::Pr => &["pr", "https://github.com/leynos/shared-actions/pull/42"],
            Self::Issue => &[
                "issue",
                "https://github.com/leynos/shared-actions/issues/42",
            ],
        }
    }

    fn response_body(self) -> &'static str {
        match self {
            Self::Pr => EMPTY_REVIEW_BODY,
            Self::Issue => ISSUE_BODY,
        }
    }
}

async fn run_vk_capture_header<F>(
    args: &[&str],
    body: &'static str,
    config_contents: &str,
    configure_cmd: F,
) -> (std::process::Output, Option<String>)
where
    F: FnOnce(&mut Command, &std::path::Path) + Send + 'static,
{
    let (addr, handler, shutdown) = start_mitm().await.expect("start server");
    let captured = Arc::new(Mutex::new(None));
    let captured_clone = captured.clone();
    let args = args.iter().map(ToString::to_string).collect::<Vec<_>>();
    *handler.lock().expect("lock handler") = Box::new(move |req: &Request<Incoming>| {
        let auth = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        *captured_clone.lock().expect("store header") = auth;
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Full::from(body))
            .expect("build response")
    });

    let config_dir = tempfile::tempdir().expect("create temp dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(&config_path, config_contents).expect("write config");

    let addr_str = format!("http://{addr}");
    let task = tokio::task::spawn_blocking(move || {
        let mut cmd = Command::cargo_bin("vk").expect("binary");
        cmd.env("GITHUB_GRAPHQL_URL", addr_str)
            .env_remove("VK_CONFIG_PATH")
            .env("VK_CONFIG_PATH", &config_path)
            .env("NO_COLOR", "1")
            .env("CLICOLOR_FORCE", "0")
            .env("RUST_LOG", "warn");
        configure_cmd(&mut cmd, &config_path);
        cmd.args(args);
        cmd.output().expect("run vk")
    });
    let output = timeout(Duration::from_secs(20), task)
        .await
        .expect("vk timed out")
        .expect("spawn blocking");
    let header = captured.lock().expect("read header").clone();
    shutdown.shutdown().await;
    (output, header)
}

async fn run_vk_command_capture_header<F>(
    cmd: VkCommand,
    config_contents: &str,
    configure_cmd: F,
) -> (std::process::Output, Option<String>)
where
    F: FnOnce(&mut Command, &std::path::Path) + Send + 'static,
{
    run_vk_capture_header(
        cmd.args(),
        cmd.response_body(),
        config_contents,
        configure_cmd,
    )
    .await
}

fn assert_token_captured(
    output: &std::process::Output,
    header: Option<&str>,
    expected_header: Option<&str>,
    expect_warning: bool,
) {
    assert!(
        output.status.success(),
        "vk exited with {:?}. Stderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    if expect_warning {
        assert!(
            stderr.contains(ANON_WARNING),
            "expected GitHub token warning: {stderr}"
        );
    } else {
        assert!(
            !stderr.contains(ANON_WARNING),
            "unexpected anonymous access warning: {stderr}"
        );
    }
    assert_eq!(header, expected_header, "authorisation header mismatch");
}

#[rstest]
#[case(true, Some("Bearer dummy"), false)]
#[case(false, None, true)]
#[tokio::test]
async fn pr_handles_authorisation(
    #[case] has_token: bool,
    #[case] expected_header: Option<&str>,
    #[case] expect_warning: bool,
) {
    let (output, header) = run_vk_command_capture_header(VkCommand::Pr, "", move |cmd, _| {
        cmd.env_remove("VK_GITHUB_TOKEN");
        if has_token {
            cmd.env("GITHUB_TOKEN", "dummy");
        } else {
            cmd.env_remove("GITHUB_TOKEN");
        }
    })
    .await;

    assert_token_captured(&output, header.as_deref(), expected_header, expect_warning);
}

#[rstest]
#[case(VkCommand::Pr, TokenSource::Config)]
#[case(VkCommand::Pr, TokenSource::VkEnv)]
#[case(VkCommand::Pr, TokenSource::Cli)]
#[case(VkCommand::Issue, TokenSource::Cli)]
#[tokio::test]
async fn vk_reads_token_from_sources(#[case] command: VkCommand, #[case] source: TokenSource) {
    let (output, header) =
        run_vk_command_capture_header(command, source.config_contents(), move |cmd, _| {
            source.configure(cmd);
        })
        .await;

    assert_token_captured(&output, header.as_deref(), Some("Bearer dummy"), false);
}

#[tokio::test]
async fn issue_warns_without_token() {
    let (output, header) = run_vk_command_capture_header(VkCommand::Issue, "", |cmd, _| {
        cmd.env_remove("GITHUB_TOKEN").env_remove("VK_GITHUB_TOKEN");
    })
    .await;

    assert_token_captured(&output, header.as_deref(), None, true);
}

#[test]
fn resolve_requires_token() {
    let config_dir = tempfile::tempdir().expect("create temp dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(&config_path, "").expect("write empty config");

    let mut cmd = Command::cargo_bin("vk").expect("binary");
    cmd.env_remove("GITHUB_TOKEN")
        .env_remove("VK_GITHUB_TOKEN")
        .env("VK_CONFIG_PATH", &config_path)
        .env("NO_COLOR", "1")
        .env("CLICOLOR_FORCE", "0")
        .args(["resolve", "https://github.com/o/r/pull/83#discussion_r1"]);

    cmd.assert()
        .failure()
        .code(2)
        .stderr(predicates::str::contains("GitHub token not set"));
}

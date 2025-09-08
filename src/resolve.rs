//! Resolve pull request review comments via the GitHub REST API.
//!
//! This module posts an optional reply then marks the comment's thread as
//! resolved. The API base URL can be overridden with the `GITHUB_API_URL`
//! environment variable for testing.

use crate::VkError;
use crate::ref_parser::RepoInfo;
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde_json::json;
use std::env;

/// Resolve a pull request review comment and optionally post a reply.
///
/// # Errors
///
/// Returns [`VkError::RequestContext`] if an HTTP request fails.
pub async fn resolve_comment(
    token: &str,
    repo: &RepoInfo,
    comment_id: u64,
    message: Option<String>,
) -> Result<(), VkError> {
    let api = env::var("GITHUB_API_URL").unwrap_or_else(|_| "https://api.github.com".into());
    let client = reqwest::Client::new();
    let base = format!(
        "{api}/repos/{owner}/{repo}/pulls/comments/{comment}",
        owner = repo.owner,
        repo = repo.name,
        comment = comment_id
    );
    let auth = format!("Bearer {token}");
    if let Some(body) = message {
        client
            .post(format!("{base}/replies"))
            .header(USER_AGENT, "vk")
            .header(AUTHORIZATION, &auth)
            .header(ACCEPT, "application/vnd.github+json")
            .json(&json!({"body": body}))
            .send()
            .await
            .map_err(|e| VkError::RequestContext {
                context: "post reply".into(),
                source: Box::new(e),
            })?
            .error_for_status()
            .map_err(|e| VkError::Request(Box::new(e)))?;
    }
    client
        .put(format!("{base}/resolve"))
        .header(USER_AGENT, "vk")
        .header(AUTHORIZATION, auth)
        .header(ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| VkError::RequestContext {
            context: "resolve comment".into(),
            source: Box::new(e),
        })?
        .error_for_status()
        .map_err(|e| VkError::Request(Box::new(e)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http_body_util::Full;
    use hyper::{Request, Response, StatusCode, body::Incoming};
    use hyper_util::rt::TokioIo;
    use hyper::server::conn::http1;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn resolve_comment_sends_requests() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_clone = Arc::clone(&calls);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let calls = Arc::clone(&calls_clone);
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let service = hyper::service::service_fn(move |req: Request<Incoming>| {
                        let calls = Arc::clone(&calls);
                        async move {
                            calls.lock().expect("lock").push(req.uri().path().to_string());
                            Ok::<_, std::convert::Infallible>(
                                Response::builder()
                                    .status(StatusCode::OK)
                                    .body(Full::new(Bytes::from_static(b"{}")))
                                    .expect("response"),
                            )
                        }
                    });
                    let _ = http1::Builder::new().serve_connection(io, service).await;
                });
            }
        });

        crate::test_utils::set_var("GITHUB_API_URL", format!("http://{addr}"));
        let repo = RepoInfo {
            owner: "o".into(),
            name: "r".into(),
        };
        resolve_comment("t", &repo, 1, Some("done".into()))
            .await
            .expect("resolve comment");
        let paths = calls.lock().expect("lock").clone();
        assert_eq!(
            paths,
            vec![
                "/repos/o/r/pulls/comments/1/replies",
                "/repos/o/r/pulls/comments/1/resolve",
            ]
        );
    }
}

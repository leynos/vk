//! Tests for review thread fetching helpers.

use super::*;
use crate::GraphQLClient;
use crate::ref_parser::RepoInfo;
use rstest::{fixture, rstest};
use tokio::task::JoinHandle;

use third_wheel::hyper::{
    Body, Request, Response, Server, StatusCode,
    service::{make_service_fn, service_fn},
};

/// Start a stub HTTP server returning each body in `responses` sequentially.
///
/// Returns a [`GraphQLClient`] targeting the server and a [`JoinHandle`] for
/// the server task.
struct TestClient {
    client: GraphQLClient,
    join: JoinHandle<()>,
}

fn start_server(responses: Vec<String>) -> TestClient {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    let responses = Arc::new(responses);
    let counter = Arc::new(AtomicUsize::new(0));
    let svc = make_service_fn(move |_conn| {
        let responses = Arc::clone(&responses);
        let counter = Arc::clone(&counter);
        async move {
            Ok::<_, std::convert::Infallible>(service_fn(move |_req: Request<Body>| {
                let idx = counter.fetch_add(1, Ordering::SeqCst);
                let body = responses
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| "{}".to_string());
                async move {
                    Ok::<_, std::convert::Infallible>(
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", "application/json")
                            .body(Body::from(body))
                            .expect("response"),
                    )
                }
            }))
        }
    });
    let server = Server::bind(&"127.0.0.1:0".parse().expect("parse addr")).serve(svc);
    let addr = server.local_addr();
    let join = tokio::spawn(async move {
        let _ = server.await;
    });
    let client = GraphQLClient::with_endpoint("token", &format!("http://{addr}"), None)
        .expect("create client");
    TestClient { client, join }
}

#[fixture]
fn repo() -> RepoInfo {
    RepoInfo {
        owner: "o".into(),
        name: "r".into(),
    }
}

#[fixture]
async fn missing_nodes_client() -> TestClient {
    let body = serde_json::json!({
        "data": {
            "repository": {
                "pullRequest": {
                    "reviewThreads": {
                        "pageInfo": { "hasNextPage": false, "endCursor": null }
                    }
                }
            }
        }
    })
    .to_string();
    start_server(vec![body])
}

fn comment(body: &str) -> serde_json::Value {
    serde_json::json!({
        "body": body,
        "diffHunk": "",
        "originalPosition": null,
        "position": null,
        "path": "f",
        "url": "",
        "author": null
    })
}

#[fixture]
async fn pagination_client() -> TestClient {
    let first: Vec<_> = (0..100).map(|i| comment(&format!("c{i}"))).collect();
    let thread_body = serde_json::json!({
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [{
                "id": "t",
                "isResolved": false,
                "comments": {"nodes": first, "pageInfo": {"hasNextPage": true, "endCursor": "c99"}}
            }],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}}
    })
    .to_string();
    let comment_body = serde_json::json!({
        "data": {"node": {"comments": {
            "nodes": [comment("c100")],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}
    })
    .to_string();
    start_server(vec![thread_body, comment_body])
}

#[rstest]
#[tokio::test]
async fn run_query_missing_nodes_reports_path(
    repo: RepoInfo,
    #[future] missing_nodes_client: TestClient,
) {
    let TestClient { client, join } = missing_nodes_client.await;
    let result = fetch_review_threads(&client, &repo, 1).await;
    let err = result.expect_err("expected error");
    let err_msg = format!("{err}");
    assert!(
        err_msg.contains("repository.pullRequest.reviewThreads"),
        "Error should contain full JSON path",
    );
    assert!(
        err_msg.contains("snippet:"),
        "Error should contain JSON snippet",
    );
    join.abort();
    let _ = join.await;
}

#[test]
fn filter_threads_by_files_retains_matches() {
    let threads = vec![
        ReviewThread {
            comments: CommentConnection {
                nodes: vec![ReviewComment {
                    path: "src/lib.rs".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        },
        ReviewThread {
            comments: CommentConnection {
                nodes: vec![ReviewComment {
                    path: "README.md".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        },
    ];
    let files = vec![String::from("README.md")];
    let filtered = filter_threads_by_files(threads, &files);
    assert_eq!(filtered.len(), 1);
    let path = filtered
        .first()
        .and_then(|t| t.comments.nodes.first())
        .map(|c| c.path.as_str());
    assert_eq!(path, Some("README.md"));
}

#[rstest]
#[tokio::test]
async fn threads_with_many_comments_do_not_duplicate_first_page(
    repo: RepoInfo,
    #[future] pagination_client: TestClient,
) {
    let TestClient { client, join } = pagination_client.await;
    let threads = fetch_review_threads(&client, &repo, 1)
        .await
        .expect("fetch threads");
    let thread = threads.first().expect("thread");
    assert_eq!(thread.comments.nodes.len(), 101);
    let bodies: Vec<_> = thread
        .comments
        .nodes
        .iter()
        .map(|c| c.body.clone())
        .collect();
    assert_eq!(
        bodies,
        (0..=100).map(|i| format!("c{i}")).collect::<Vec<_>>()
    );
    join.abort();
    let _ = join.await;
}

//! Tests for review thread fetching helpers.

use super::*;
use crate::GraphQLClient;
use crate::ref_parser::RepoInfo;

#[tokio::test]
async fn run_query_missing_nodes_reports_path() {
    use third_wheel::hyper::{
        Body, Request, Response, Server, StatusCode,
        service::{make_service_fn, service_fn},
    };

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

    let make_svc = make_service_fn(move |_conn| {
        let body = body.clone();
        async move {
            Ok::<_, std::convert::Infallible>(service_fn(move |_req: Request<Body>| {
                let body = body.clone();
                async move {
                    Ok::<_, std::convert::Infallible>(
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", "application/json")
                            .body(Body::from(body.clone()))
                            .expect("failed to build HTTP response"),
                    )
                }
            }))
        }
    });

    let server = Server::bind(
        &"127.0.0.1:0"
            .parse()
            .expect("failed to parse server address"),
    )
    .serve(make_svc);
    let addr = server.local_addr();
    let join = tokio::spawn(server);

    let client = GraphQLClient::with_endpoint("token", &format!("http://{addr}"), None)
        .expect("failed to create GraphQL client");

    let repo = RepoInfo {
        owner: "o".into(),
        name: "r".into(),
    };
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

#[tokio::test]
async fn threads_with_many_comments_do_not_duplicate_first_page() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use third_wheel::hyper::{
        Body, Request, Response, Server, StatusCode,
        service::{make_service_fn, service_fn},
    };

    fn comment(body: &str) -> serde_json::Value {
        json!({
            "body": body,
            "diffHunk": "",
            "originalPosition": null,
            "position": null,
            "path": "f",
            "url": "",
            "author": null
        })
    }

    let first: Vec<_> = (0..100).map(|i| comment(&format!("c{i}"))).collect();
    let thread_body = json!({
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
    let comment_body = json!({
        "data": {"node": {"comments": {
            "nodes": [comment("c100")],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}
    })
    .to_string();

    let counter = Arc::new(AtomicUsize::new(0));
    let svc = make_service_fn(move |_conn| {
        let thread_body = thread_body.clone();
        let comment_body = comment_body.clone();
        let counter = Arc::clone(&counter);
        async move {
            Ok::<_, std::convert::Infallible>(service_fn(move |_req: Request<Body>| {
                let body = if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    thread_body.clone()
                } else {
                    comment_body.clone()
                };
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

    let server = Server::bind(
        &"127.0.0.1:0"
            .parse()
            .expect("failed to parse server address"),
    )
    .serve(svc);
    let addr = server.local_addr();
    let join = tokio::spawn(server);
    let client = GraphQLClient::with_endpoint("token", &format!("http://{addr}"), None)
        .expect("failed to create client");
    let repo = RepoInfo {
        owner: "o".into(),
        name: "r".into(),
    };
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

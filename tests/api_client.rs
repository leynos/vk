mod utils;
use crate::utils::start_mitm;
use http_body_util::Full;
use hyper::{Response, StatusCode};
use vk::{GraphQLClient, RepoInfo, fetch_review_threads};

#[tokio::test]
async fn run_query_missing_nodes_reports_path() {
    let body = serde_json::json!({
        "data": {
            "repository": {
                "pullRequest": {"reviewThreads": {"pageInfo": {"hasNextPage": false, "endCursor": null }}}
            }
        }
    }).to_string();

    let (addr, handler, shutdown) = start_mitm().await.expect("mitm");
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Full::from(body.clone()))
            .expect("resp")
    });

    let client =
        GraphQLClient::with_endpoint("token", &format!("http://{addr}"), None).expect("client");
    let repo = RepoInfo {
        owner: "o".into(),
        name: "r".into(),
    };
    let result = fetch_review_threads(&client, &repo, 1).await;
    let err = result.expect_err("expected error");
    let err_msg = format!("{err}");
    assert!(err_msg.contains("repository.pullRequest.reviewThreads"));
    assert!(err_msg.contains("snippet:"));

    shutdown.shutdown().await;
}

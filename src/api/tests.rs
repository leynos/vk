//! Tests for API utilities.

use super::*;
use crate::PageInfo;
use rstest::{fixture, rstest};
use serde_json::{Map, Value, json};
use std::{
    cell::RefCell,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use third_wheel::hyper::{
    Body, Request, Response, Server, StatusCode,
    service::{make_service_fn, service_fn},
};
use tokio::{task::JoinHandle, time::Duration};

struct TestClient {
    client: GraphQLClient,
    join: JoinHandle<()>,
}

fn start_server(responses: Vec<String>) -> TestClient {
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
    let retry = RetryConfig {
        base_delay: Duration::from_millis(1),
        ..RetryConfig::default()
    };
    let client = GraphQLClient::with_endpoint_retry("token", format!("http://{addr}"), None, retry)
        .expect("create client");
    TestClient { client, join }
}

#[fixture]
fn mock_server_with_capture() -> (GraphQLClient, Arc<Mutex<String>>, JoinHandle<()>) {
    use third_wheel::hyper::body::to_bytes;

    let captured = Arc::new(Mutex::new(String::new()));
    let cap_clone = Arc::clone(&captured);
    let svc = make_service_fn(move |_conn| {
        let cap_inner = Arc::clone(&cap_clone);
        async move {
            Ok::<_, std::convert::Infallible>(service_fn(move |req: Request<Body>| {
                let cap = Arc::clone(&cap_inner);
                async move {
                    let bytes = to_bytes(req.into_body()).await.expect("body");
                    *cap.lock().expect("lock") = String::from_utf8(bytes.to_vec()).expect("utf8");
                    Ok::<_, std::convert::Infallible>(
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", "application/json")
                            .body(Body::from("{\"data\":{}}"))
                            .expect("response"),
                    )
                }
            }))
        }
    });
    let server = Server::bind(&"127.0.0.1:0".parse().expect("addr")).serve(svc);
    let addr = server.local_addr();
    let join = tokio::spawn(async move {
        let _ = server.await;
    });
    let client =
        GraphQLClient::with_endpoint("token", format!("http://{addr}"), None).expect("client");

    (client, captured, join)
}

fn assert_cursor_in_request(captured: &Arc<Mutex<String>>, expected: &str) {
    let body = captured.lock().expect("lock").to_string();
    let v: Value = serde_json::from_str(&body).expect("json body");
    let cur = v
        .get("variables")
        .and_then(|vars| vars.get("cursor"))
        .and_then(Value::as_str);
    assert_eq!(cur, Some(expected));
}

#[tokio::test]
async fn run_query_retries_missing_data() {
    let responses = vec![
        "{}".to_string(),
        serde_json::json!({"data": {"x": 1}}).to_string(),
    ];
    let TestClient { client, join } = start_server(responses);
    let result: serde_json::Value = client
        .run_query("query", serde_json::json!({}))
        .await
        .expect("success");
    assert_eq!(result, serde_json::json!({"x": 1}));
    join.abort();
    let _ = join.await;
}

#[tokio::test]
async fn fetch_page_rejects_non_object_variables() {
    let client = GraphQLClient::with_endpoint("token", "http://127.0.0.1:9", None).expect("client");
    let err = client
        .fetch_page::<Value, _>("query", None, serde_json::json!(null))
        .await
        .expect_err("error");
    match err {
        VkError::BadResponse(msg) => {
            assert!(msg.contains("variables for fetch_page must be a JSON object"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[rstest]
#[case(Map::new(), "abc", "abc")]
#[case({
    let mut vars = Map::new();
    vars.insert("cursor".into(), json!("stale"));
    vars
}, "fresh", "fresh")]
#[tokio::test]
async fn fetch_page_cursor_handling(
    mock_server_with_capture: (GraphQLClient, Arc<Mutex<String>>, JoinHandle<()>),
    #[case] variables: Map<String, Value>,
    #[case] cursor: &str,
    #[case] expected: &str,
) {
    let (client, captured, join) = mock_server_with_capture;
    let _: Value = client
        .fetch_page("query", Some(cursor.into()), variables)
        .await
        .expect("fetch");
    join.abort();
    let _ = join.await;
    assert_cursor_in_request(&captured, expected);
}

#[tokio::test]
async fn paginate_discards_items_on_error() {
    let seen = RefCell::new(Vec::new());

    let result: Result<Vec<i32>, VkError> = paginate(|cursor| {
        let seen = &seen;
        async move {
            if cursor.is_none() {
                seen.borrow_mut().push(1);
                Ok((
                    vec![1],
                    PageInfo {
                        has_next_page: true,
                        end_cursor: Some("next".to_string()),
                    },
                ))
            } else {
                Err(VkError::ApiErrors("boom".into()))
            }
        }
    })
    .await;

    assert!(result.is_err());
    assert_eq!(seen.borrow().as_slice(), &[1]);
}

#[tokio::test]
async fn paginate_missing_cursor_errors() {
    let result: Result<Vec<i32>, VkError> = paginate(|_cursor| async {
        Ok((
            vec![1],
            PageInfo {
                has_next_page: true,
                end_cursor: None,
            },
        ))
    })
    .await;
    match result {
        Err(VkError::BadResponse(msg)) => {
            let s = msg.to_string();
            assert!(
                s.contains("hasNextPage=true") && s.contains("endCursor"),
                "{s}"
            );
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

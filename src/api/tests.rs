//! Tests for API utilities.

use super::*;
use crate::PageInfo;
use rstest::{fixture, rstest};
use serde_json::{Map, Value, json};
use std::{
    borrow::Cow,
    cell::RefCell,
    convert::Infallible,
    future::Future,
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

fn create_test_server<F, Fut>(
    response_handler: F,
) -> (GraphQLClient, JoinHandle<()>, Arc<AtomicUsize>)
where
    F: Fn(usize) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Response<Body>, Infallible>> + Send + 'static,
{
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = Arc::clone(&counter);
    let handler = Arc::new(response_handler);
    let svc = make_service_fn(move |_conn| {
        let counter = Arc::clone(&counter_clone);
        let handler = Arc::clone(&handler);
        async move {
            Ok::<_, Infallible>(service_fn(move |_req: Request<Body>| {
                let handler = Arc::clone(&handler);
                let idx = counter.fetch_add(1, Ordering::SeqCst);
                (*handler)(idx)
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
        jitter: false,
        ..RetryConfig::default()
    };
    let client = GraphQLClient::with_endpoint_retry("token", format!("http://{addr}"), None, retry)
        .expect("create client");
    (client, join, counter)
}

fn start_server(responses: Vec<String>) -> TestClient {
    start_server_with_status(responses, StatusCode::OK)
}

fn start_server_with_status(responses: Vec<String>, status: StatusCode) -> TestClient {
    let responses = Arc::new(responses);
    let handler = move |idx: usize| {
        let responses = Arc::clone(&responses);
        async move {
            let body = responses
                .get(idx)
                .cloned()
                .unwrap_or_else(|| "{}".to_string());
            let content_type = if body.trim_start().starts_with('<') {
                "text/html; charset=utf-8"
            } else {
                "application/json; charset=utf-8"
            };
            Ok::<_, Infallible>(
                Response::builder()
                    .status(status)
                    .header("Content-Type", content_type)
                    .body(Body::from(body))
                    .expect("response"),
            )
        }
    };
    let (client, join, _) = create_test_server(handler);
    TestClient { client, join }
}

#[derive(Clone)]
struct RespSpec {
    status: StatusCode,
    body: String,
}

fn start_server_sequence(specs: Vec<RespSpec>) -> TestClient {
    let specs = Arc::new(specs);
    let handler = move |idx: usize| {
        let specs = Arc::clone(&specs);
        async move {
            let RespSpec { status, body } = specs.get(idx).cloned().unwrap_or_else(|| RespSpec {
                status: StatusCode::OK,
                body: "{}".into(),
            });
            Ok::<_, Infallible>(
                Response::builder()
                    .status(status)
                    .header("Content-Type", "application/json; charset=utf-8")
                    .body(Body::from(body))
                    .expect("response"),
            )
        }
    };
    let (client, join, _) = create_test_server(handler);
    TestClient { client, join }
}

#[derive(Clone, Debug)]
struct ScriptedResp {
    status: StatusCode,
    body: String,
    content_type: &'static str,
}

fn start_server_scripted(
    script: Vec<ScriptedResp>,
) -> (GraphQLClient, JoinHandle<()>, Arc<AtomicUsize>) {
    let responses = Arc::new(script);
    let handler = move |idx: usize| {
        let responses = Arc::clone(&responses);
        async move {
            let resp = responses.get(idx).cloned().unwrap_or_else(|| ScriptedResp {
                status: StatusCode::OK,
                body: "{}".to_string(),
                content_type: "application/json; charset=utf-8",
            });
            Ok::<_, Infallible>(
                Response::builder()
                    .status(resp.status)
                    .header("Content-Type", resp.content_type)
                    .body(Body::from(resp.body))
                    .expect("response"),
            )
        }
    };
    create_test_server(handler)
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
                            .header("Content-Type", "application/json; charset=utf-8")
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
        .run_query("query RetryOp { __typename }", serde_json::json!({}))
        .await
        .expect("success");
    assert_eq!(result, serde_json::json!({"x": 1}));
    join.abort();
    let _ = join.await;
}

#[tokio::test]
async fn run_query_retries_on_5xx_then_succeeds() {
    let specs = vec![
        RespSpec {
            status: StatusCode::BAD_GATEWAY,
            body: "<html>bad gateway</html>".into(),
        },
        RespSpec {
            status: StatusCode::OK,
            body: serde_json::json!({"data": {"x": 1}}).to_string(),
        },
    ];
    let TestClient { client, join } = start_server_sequence(specs);
    let result: Value = client
        .run_query("query OkAfter { __typename }", serde_json::json!({}))
        .await
        .expect("ok");
    assert_eq!(result, serde_json::json!({"x": 1}));
    join.abort();
    let _ = join.await;
}

#[tokio::test]
async fn run_query_retries_html_5xx_then_succeeds() {
    let script = vec![
        ScriptedResp {
            status: StatusCode::BAD_GATEWAY,
            body: "<html>bad gateway</html>".into(),
            content_type: "text/html; charset=utf-8",
        },
        ScriptedResp {
            status: StatusCode::OK,
            body: serde_json::json!({"data": {"x": 1}}).to_string(),
            content_type: "application/json; charset=utf-8",
        },
    ];
    let (client, join, hits) = start_server_scripted(script);
    let result: Value = client
        .run_query("query HtmlRetry { __typename }", serde_json::json!({}))
        .await
        .expect("success after retry");
    assert_eq!(result, serde_json::json!({"x": 1}));
    assert!(hits.load(Ordering::SeqCst) >= 2, "expected at least 2 hits");
    join.abort();
    let _ = join.await;
}

#[tokio::test]
async fn run_query_includes_operation_name() {
    let (client, captured, join) = mock_server_with_capture();
    let _: Value = client
        .run_query("query MyOp { __typename }", json!({}))
        .await
        .expect("ok");
    join.abort();
    let _ = join.await;

    let body = captured.lock().expect("lock").to_string();
    let v: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v.get("operationName").and_then(Value::as_str), Some("MyOp"));
}

#[derive(Debug)]
struct OperationNameCase {
    query: &'static str,
    expected: Option<&'static str>,
}

#[rstest]
#[case(OperationNameCase {
    query: "query RetryOp { __typename }",
    expected: Some("RetryOp"),
})]
#[case(OperationNameCase {
    query: "queryFoo { __typename }",
    expected: None,
})]
#[tokio::test]
async fn run_query_operation_name_handling(
    mock_server_with_capture: (GraphQLClient, Arc<Mutex<String>>, JoinHandle<()>),
    #[case] case: OperationNameCase,
) {
    let (client, captured, join) = mock_server_with_capture;
    let _: Value = client
        .run_query(case.query, serde_json::json!({}))
        .await
        .expect("ok");
    join.abort();
    let _ = join.await;

    let body = captured.lock().expect("lock").to_string();
    let v: Value = serde_json::from_str(&body).expect("json body");
    let op = v.get("operationName").and_then(Value::as_str);
    assert_eq!(op, case.expected);
}

#[derive(Debug)]
struct TestCase {
    responses: Vec<String>,
    status: StatusCode,
    op: &'static str,
    expect: Expected,
}

#[derive(Debug)]
enum Expected {
    EmptyResponse { fragments: [&'static str; 3] },
    ApiErrors { fragment: &'static str },
    RequestCtx { fragments: [&'static str; 2] },
}

#[rstest]
#[case(TestCase {
    responses: vec![],
    status: StatusCode::OK,
    op: "query EmptyOp { }",
    expect: Expected::EmptyResponse {
        fragments: ["status 200", "EmptyOp", "{}"],
    },
})]
#[case(TestCase {
    responses: vec![],
    status: StatusCode::INTERNAL_SERVER_ERROR,
    op: "query FailOp { }",
    expect: Expected::RequestCtx {
        fragments: ["status 500", "body snippet: {}"],
    },
})]
#[case({
    let error_response = serde_json::json!({
        "errors": [
            { "message": "Something went wrong", "locations": [{ "line": 1, "column": 2 }] }
        ]
    })
    .to_string();
    TestCase {
        responses: vec![error_response],
        status: StatusCode::OK,
        op: "query ErrOp { }",
        expect: Expected::ApiErrors {
            fragment: "Something went wrong",
        },
    }
})]
#[case(TestCase {
    responses: vec![],
    status: StatusCode::TOO_MANY_REQUESTS,
    op: "query RateLimited { }",
    expect: Expected::RequestCtx {
        fragments: ["status 429", "body snippet: {}"],
    },
})]
#[tokio::test]
async fn run_query_reports_details(#[case] case: TestCase) {
    let TestCase {
        responses,
        status,
        op,
        expect,
    } = case;
    let TestClient { client, join } = start_server_with_status(responses, status);
    let err = client
        .run_query::<_, Value>(op, serde_json::json!({}))
        .await
        .expect_err("error");
    match expect {
        Expected::EmptyResponse { fragments } => match &err {
            VkError::EmptyResponse { .. } => {
                let s = err.to_string();
                for frag in fragments {
                    assert!(s.contains(frag), "{s}");
                }
            }
            other => panic!("unexpected error: {other:?}"),
        },
        Expected::ApiErrors { fragment } => match err {
            VkError::ApiErrors(msg) => {
                assert!(msg.contains(fragment), "{msg}");
            }
            other => panic!("unexpected error: {other:?}"),
        },
        Expected::RequestCtx { fragments } => match err {
            VkError::RequestContext { .. } => {
                let s = err.to_string();
                for frag in fragments {
                    assert!(s.contains(frag), "{s}");
                }
            }
            other => panic!("unexpected error: {other:?}"),
        },
    }
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
#[case(false, Map::new(), "abc", "abc")]
#[case(true, Map::new(), "abc", "abc")]
#[case(false, {
    let mut vars = Map::new();
    vars.insert("cursor".into(), json!("stale"));
    vars
}, "fresh", "fresh")]
#[case(true, {
    let mut vars = Map::new();
    vars.insert("cursor".into(), json!("stale"));
    vars
}, "fresh", "fresh")]
#[tokio::test]
async fn fetch_page_cursor_handling_param(
    mock_server_with_capture: (GraphQLClient, Arc<Mutex<String>>, JoinHandle<()>),
    #[case] owned: bool,
    #[case] variables: Map<String, Value>,
    #[case] cursor: &str,
    #[case] expected: &str,
) {
    let (client, captured, join) = mock_server_with_capture;
    let _: Value = if owned {
        client
            .fetch_page("query", Some(Cow::Owned(cursor.to_string())), variables)
            .await
            .expect("fetch")
    } else {
        client
            .fetch_page("query", Some(Cow::Borrowed(cursor)), variables)
            .await
            .expect("fetch")
    };
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

#[rstest]
#[case(false, None, None)]
#[case(true, Some(String::from("abc")), Some("abc"))]
fn next_cursor_ok_cases(
    #[case] has_next_page: bool,
    #[case] end_cursor: Option<String>,
    #[case] expected: Option<&str>,
) {
    let info = PageInfo {
        has_next_page,
        end_cursor,
    };
    let next = info.next_cursor().expect("cursor");
    match (next, expected) {
        (None, None) => {}
        (Some(got), Some(want)) => assert_eq!(got, want),
        other => panic!("unexpected case: {other:?}"),
    }
}

#[test]
fn next_cursor_errors_without_cursor() {
    let info = PageInfo {
        has_next_page: true,
        end_cursor: None,
    };
    let err = info.next_cursor().expect_err("missing cursor");
    assert!(matches!(err, VkError::BadResponse(_)));
}

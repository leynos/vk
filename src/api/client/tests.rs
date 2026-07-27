//! Tests for GraphQL client behaviour.

use super::*;
use crate::VkError;
use crate::api::RetryConfig;
use rstest::rstest;
use serde_json::Value;
use std::{
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
fn start_server_generic<F>(handler: F) -> TestClient
where
    F: Fn(usize) -> (StatusCode, String) + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    let svc = move |idx: usize| {
        let handler = Arc::clone(&handler);
        async move {
            let (status, body) = handler(idx);
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
    let (client, join, _) = create_test_server(svc);
    TestClient { client, join }
}
fn start_server(responses: Vec<String>) -> TestClient {
    start_server_with_status(responses, StatusCode::OK)
}
fn start_server_with_status(responses: Vec<String>, status: StatusCode) -> TestClient {
    let responses = Arc::new(responses);
    start_server_generic(move |idx| {
        let body = responses
            .get(idx)
            .cloned()
            .unwrap_or_else(|| "{}".to_string());
        (status, body)
    })
}
#[derive(Clone, Debug)]
struct RespSpec {
    status: StatusCode,
    body: String,
}
fn start_server_sequence(specs: Vec<RespSpec>) -> TestClient {
    let specs = Arc::new(specs);
    start_server_generic(move |idx| {
        let RespSpec { status, body } = specs.get(idx).cloned().unwrap_or_else(|| RespSpec {
            status: StatusCode::OK,
            body: "{}".into(),
        });
        (status, body)
    })
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
/// Start a stub server that captures the last request body and replies with
/// an empty `data` object. Shared with the pagination test module, which uses
/// it to assert the cursor a typed operation sends on the wire.
pub(super) fn mock_server_with_capture() -> (GraphQLClient, Arc<Mutex<String>>, JoinHandle<()>) {
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
/// Assert the captured request body carries `variables.cursor == expected`.
pub(super) fn assert_cursor_in_request(captured: &Arc<Mutex<String>>, expected: &str) {
    let body = captured.lock().expect("lock").to_string();
    let v: Value = serde_json::from_str(&body).expect("json body");
    let cur = v
        .get("variables")
        .and_then(|vars| vars.get("cursor"))
        .and_then(Value::as_str);
    assert_eq!(cur, Some(expected));
}
/// Build the `{"query", "variables", "operationName"}` envelope the retired
/// string-based `run_query` produced, for driving [`GraphQLClient::run_payload`]
/// directly in the characterization tests below.
fn payload_for(op_name: &str) -> Value {
    serde_json::json!({
        "query": format!("query {op_name} {{ __typename }}"),
        "variables": {},
        "operationName": op_name,
    })
}
#[tokio::test]
async fn run_payload_retries_missing_data() {
    let responses = vec![
        "{}".to_string(),
        serde_json::json!({"data": {"x": 1}}).to_string(),
    ];
    let TestClient { client, join } = start_server(responses);
    let result: serde_json::Value = client
        .run_payload(&payload_for("RetryOp"), "RetryOp")
        .await
        .expect("success");
    assert_eq!(result, serde_json::json!({"x": 1}));
    join.abort();
    let _ = join.await;
}
#[tokio::test]
async fn run_payload_retries_on_5xx_then_succeeds() {
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
        .run_payload(&payload_for("OkAfter"), "OkAfter")
        .await
        .expect("ok");
    assert_eq!(result, serde_json::json!({"x": 1}));
    join.abort();
    let _ = join.await;
}
#[tokio::test]
async fn run_payload_retries_html_5xx_then_succeeds() {
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
        .run_payload(&payload_for("HtmlRetry"), "HtmlRetry")
        .await
        .expect("success after retry");
    assert_eq!(result, serde_json::json!({"x": 1}));
    assert!(hits.load(Ordering::SeqCst) >= 2, "expected at least 2 hits");
    join.abort();
    let _ = join.await;
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
    op: "EmptyOp",
    expect: Expected::EmptyResponse {
        fragments: ["status 200", "EmptyOp", "{}"],
    },
})]
#[case(TestCase {
    responses: vec![],
    status: StatusCode::INTERNAL_SERVER_ERROR,
    op: "FailOp",
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
        op: "ErrOp",
        expect: Expected::ApiErrors {
            fragment: "Something went wrong",
        },
    }
})]
#[case(TestCase {
    responses: vec![],
    status: StatusCode::TOO_MANY_REQUESTS,
    op: "RateLimited",
    expect: Expected::RequestCtx {
        fragments: ["status 429", "body snippet: {}"],
    },
})]
#[tokio::test]
async fn run_payload_reports_details(#[case] case: TestCase) {
    let TestCase {
        responses,
        status,
        op,
        expect,
    } = case;
    let TestClient { client, join } = start_server_with_status(responses, status);
    let err = client
        .run_payload::<Value>(&payload_for(op), op)
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
// NOTE: the string-based `fetch_page` and its non-object-variables guard were
// removed with the `run_query` surface; typed `Variables` structs are objects
// by construction, so that failure mode no longer exists. Cursor handling is
// characterized by `paginate_operation_overwrites_stale_cursor_in_request` in
// the pagination test module, which asserts the cursor on the wire exactly as
// the retired `fetch_page` tests did.

//! Response-classification tests for the GraphQL client.

use super::{
    Value, VkError,
    tests::{TestClient, start_server_with_status},
};
use rstest::rstest;
use third_wheel::hyper::StatusCode;

#[derive(Debug)]
struct TestCase {
    responses: Vec<String>,
    status: StatusCode,
    operation: &'static str,
    expected: Expected,
}

#[derive(Debug)]
enum Expected {
    EmptyResponse { fragments: [&'static str; 3] },
    ApiErrors { fragment: &'static str },
    RequestContext { fragments: [&'static str; 2] },
}

#[rstest]
#[case(TestCase {
    responses: vec![],
    status: StatusCode::OK,
    operation: "query EmptyOp { }",
    expected: Expected::EmptyResponse {
        fragments: ["status 200", "EmptyOp", "{}"],
    },
})]
#[case(TestCase {
    responses: vec![],
    status: StatusCode::INTERNAL_SERVER_ERROR,
    operation: "query FailOp { }",
    expected: Expected::RequestContext {
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
        operation: "query ErrOp { }",
        expected: Expected::ApiErrors {
            fragment: "Something went wrong",
        },
    }
})]
#[case(TestCase {
    responses: vec![],
    status: StatusCode::TOO_MANY_REQUESTS,
    operation: "query RateLimited { }",
    expected: Expected::RequestContext {
        fragments: ["status 429", "body snippet: {}"],
    },
})]
#[tokio::test]
async fn run_query_reports_details(#[case] case: TestCase) {
    let TestCase {
        responses,
        status,
        operation,
        expected,
    } = case;
    let TestClient { client, join } = start_server_with_status(responses, status);
    let error = client
        .run_query::<_, Value>(operation, serde_json::json!({}))
        .await
        .expect_err("response should fail");
    match expected {
        Expected::EmptyResponse { fragments } => match &error {
            VkError::EmptyResponse { .. } => {
                let diagnostic = error.to_string();
                for fragment in fragments {
                    assert!(diagnostic.contains(fragment), "{diagnostic}");
                }
            }
            other => panic!("unexpected error: {other:?}"),
        },
        Expected::ApiErrors { fragment } => match error {
            VkError::ApiErrors(message) => assert!(message.contains(fragment), "{message}"),
            other => panic!("unexpected error: {other:?}"),
        },
        Expected::RequestContext { fragments } => match &error {
            VkError::RequestContext { .. } => {
                let diagnostic = error.to_string();
                for fragment in fragments {
                    assert!(diagnostic.contains(fragment), "{diagnostic}");
                }
            }
            other => panic!("unexpected error: {other:?}"),
        },
    }
    join.abort();
    let _ = join.await;
}

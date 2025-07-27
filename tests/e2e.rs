use assert_cmd::Command;
use predicates::str::contains;
use serde_json::Value;
use std::{
    fs,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use third_wheel::{
    CertificateAuthority, MitmProxy, ThirdWheel,
    hyper::{Body, Request, Response, StatusCode},
    mitm_layer,
};

// Shared handler type for dynamic responses
type Handler = Arc<Mutex<Box<dyn FnMut(&Request<Body>) -> Response<Body> + Send>>>;

fn start_mock_server() -> (SocketAddr, Handler) {
    let handler: Handler = Arc::new(Mutex::new(Box::new(|_req| {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("No handler"))
            .expect("build response")
    })));
    let handler_clone = handler.clone();
    let ca = CertificateAuthority::load_from_pem_files_with_passphrase_on_key(
        "tests/ca/cert.pem",
        "tests/ca/key.pem",
        "third-wheel",
    )
    .expect("load ca");
    let mitm = mitm_layer(move |req: Request<Body>, _tw: ThirdWheel| {
        let mut h = handler_clone.lock().expect("lock handler");
        let resp = (*h)(&req);
        Box::pin(async move { Ok(resp) })
    });
    let proxy = MitmProxy::builder(mitm, ca).build();
    let (addr, fut) = proxy.bind("127.0.0.1:0".parse().expect("parse addr"));
    tokio::spawn(fut);
    (addr, handler)
}

fn load_transcript(path: &str) -> Vec<String> {
    let data = fs::read_to_string(path).expect("read transcript");
    data.lines()
        .map(|line| {
            let v: Value = serde_json::from_str(line).expect("valid json line");
            v.get("response")
                .and_then(|r| r.as_str())
                .unwrap_or("{}")
                .to_owned()
        })
        .collect()
}

#[tokio::test]
#[ignore = "requires recorded network transcript"]
async fn e2e_pr_42() {
    let (addr, handler) = start_mock_server();
    let mut responses = load_transcript("tests/fixtures/pr42.json").into_iter();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = responses.next().unwrap_or_else(|| "{}".to_string());
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .expect("build response")
    });

    Command::cargo_bin("vk")
        .expect("binary")
        .env("GITHUB_GRAPHQL_URL", format!("http://{addr}"))
        .env("GITHUB_TOKEN", "dummy")
        .args(["pr", "https://github.com/leynos/shared-actions/pull/42"])
        .assert()
        .success()
        .stdout(contains("end of code review"));
}

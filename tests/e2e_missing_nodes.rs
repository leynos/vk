use assert_cmd::Command;
use predicates::str::contains;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use third_wheel::{
    CertificateAuthority, MitmProxy, ThirdWheel,
    hyper::{Body, Request, Response, StatusCode},
    mitm_layer,
};

type Handler = Arc<Mutex<Box<dyn FnMut(&Request<Body>) -> Response<Body> + Send>>>;

fn generate_ca() -> CertificateAuthority {
    use std::process::Command as PCommand;
    let dir = tempfile::tempdir().expect("tempdir");
    let cert = dir.path().join("cert.pem");
    let key = dir.path().join("key.pem");
    let status = PCommand::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:4096",
            "-keyout",
            key.to_str().expect("path"),
            "-out",
            cert.to_str().expect("path"),
            "-days",
            "1",
            "-passout",
            "pass:third-wheel",
            "-subj",
            "/C=US/ST=test/L=test/O=vk/CN=vk.test",
        ])
        .status()
        .expect("run openssl");
    assert!(status.success(), "openssl failed");
    CertificateAuthority::load_from_pem_files_with_passphrase_on_key(cert, key, "third-wheel")
        .expect("load ca")
}

fn start_mock_server() -> (SocketAddr, Handler) {
    let handler: Handler = Arc::new(Mutex::new(Box::new(|_req| {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("No handler"))
            .expect("build response")
    })));
    let handler_clone = handler.clone();
    let ca = generate_ca();
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

#[tokio::test]
async fn e2e_missing_nodes_reports_path() {
    let (addr, handler) = start_mock_server();
    *handler.lock().expect("lock handler") = Box::new(move |_req| {
        let body = serde_json::json!({
            "data": {
                "repository": {
                    "pullRequest": {
                        "reviewThreads": {
                            "pageInfo": {
                                "hasNextPage": false,
                                "endCursor": null
                            }
                        }
                    }
                }
            }
        })
        .to_string();
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
        .args(["pr", "https://github.com/leynos/cmd-mox/pull/25"])
        .assert()
        .failure()
        .stderr(contains("repository.pullRequest.reviewThreads"))
        .stderr(contains("snippet:"));
}

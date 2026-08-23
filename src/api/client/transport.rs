//! Direct hyper transport for GraphQL requests.
//!
//! Replaces the former reqwest client with a pooled `hyper_util` legacy client
//! layered over a `hyper_rustls` HTTPS connector. This reassembles what reqwest
//! provided for free — connection pooling, rustls TLS, a total-request timeout,
//! and body collection — while leaving the surrounding transcript, retry, and
//! error-context machinery in [`super::GraphQLClient`] untouched.
//!
//! Deliberate behavioural differences from the reqwest client it replaces:
//!
//! - System proxies are not honoured. reqwest read `HTTP(S)_PROXY` from the
//!   environment by default; a bare hyper client does not, and this transport
//!   intentionally does not restore that. GraphQL traffic to GitHub (or a
//!   loopback test server) does not traverse a proxy in any supported
//!   deployment, so this is an accepted, untested blind spot.
//! - Redirects are not followed. reqwest followed up to ten redirects by
//!   default; the GraphQL POST target never issues one in practice, so a 3xx
//!   would surface as a non-2xx error exactly like any other unexpected status.

use std::time::{Duration, Instant};

use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderValue, Method, Request, Uri};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde_json::Value;

use super::helpers::{operation_name, payload_snippet, snippet};
use super::http::HttpResponse;
use super::types::Endpoint;
use crate::VkError;
use crate::boxed::BoxedStr;

/// Owns the pooled hyper client used for GraphQL requests.
pub(super) struct Transport {
    /// Pooled client shared by every GraphQL request.
    client: Client<HttpsConnector<HttpConnector>, Full<Bytes>>,
}

/// Inputs for one JSON POST through the GraphQL transport.
///
/// Grouping the request data keeps the transport boundary cohesive as the
/// caller preserves the existing GraphQL request contract.
pub(super) struct PostJsonRequest<'a> {
    /// GraphQL endpoint to receive the request.
    pub(super) endpoint: &'a Endpoint,
    /// Headers applied to the request.
    pub(super) headers: &'a HeaderMap,
    /// JSON payload serialized into the request body.
    pub(super) payload: &'a Value,
    /// Deadline covering request submission and body collection.
    pub(super) timeout: Duration,
}

impl Transport {
    /// Build a transport with a pooled client over an HTTPS/HTTP connector.
    ///
    /// The connector serves both `https://` (production) and `http://` (the
    /// loopback stub servers used by the test suite) URLs. Its root store is
    /// Mozilla's webpki roots with the ring crypto provider, matching reqwest's
    /// former `rustls-tls` feature.
    ///
    /// # Errors
    ///
    /// Returns a [`VkError`] to preserve a fallible construction contract for a
    /// future extraction into a shared crate, where TLS provider selection may
    /// become fallible; the current webpki/ring setup cannot fail.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "fallible signature is part of the documented transport contract; \
                  webpki-roots setup happens to be infallible today"
    )]
    pub(super) fn new() -> Result<Self, VkError> {
        let connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http1()
            .build();
        let client = Client::builder(TokioExecutor::new()).build(connector);
        Ok(Self { client })
    }

    /// Send a JSON POST request and return its status and body.
    ///
    /// The `timeout` bounds the entire exchange (connection, send, and body
    /// collection), mirroring reqwest's `.timeout()` scope. A timeout maps to a
    /// [`VkError::RequestContext`], which the retry classifier treats as
    /// transient, exactly as a reqwest timeout was treated. Transport and
    /// body-collection failures reproduce the same context strings the reqwest
    /// client built.
    ///
    /// # Errors
    ///
    /// Returns a [`VkError::RequestContext`] if the endpoint URL is invalid, the
    /// request cannot be built or sent, the request times out, or the response
    /// body cannot be collected.
    pub(super) async fn post_json(
        &self,
        request: PostJsonRequest<'_>,
    ) -> Result<HttpResponse, VkError> {
        let PostJsonRequest {
            endpoint,
            headers,
            payload,
            timeout,
        } = request;
        // Reconstruct the same request-context strings the reqwest client
        // produced: the operation name (or a query snippet fallback) and a
        // redacted payload snippet. Derived from the payload so this transport
        // stays self-contained behind its fixed signature.
        let snip = payload_snippet(payload);
        let query = payload.get("query").and_then(Value::as_str).unwrap_or("");
        let operation = operation_name(query).map_or_else(|| snippet(query, 64), str::to_string);
        let make_ctx = |status: Option<u16>| {
            let base = format!("operation {operation}; {snip}");
            // Disambiguate from `BodyExt::boxed`, which is also in scope.
            BoxedStr::boxed(match status {
                Some(s) => format!("{base}; status {s}"),
                None => base,
            })
        };

        let request = build_request(endpoint, headers, payload).map_err(|source| {
            VkError::RequestContext {
                context: make_ctx(None),
                source,
            }
        })?;

        let started_at = Instant::now();
        let response = tokio::time::timeout(timeout, self.client.request(request))
            .await
            .map_err(|_| request_timeout_error(make_ctx(None), timeout))?
            .map_err(|e| VkError::RequestContext {
                context: make_ctx(None),
                source: Box::new(e),
            })?;
        let status = response.status().as_u16();
        let remaining = timeout.saturating_sub(started_at.elapsed());
        let collected = tokio::time::timeout(remaining, response.into_body().collect())
            .await
            .map_err(|_| request_timeout_error(make_ctx(Some(status)), timeout))?
            .map_err(|e| VkError::RequestContext {
                context: make_ctx(Some(status)),
                source: Box::new(e),
            })?;
        // Lossily decode, matching reqwest's UTF-8 text handling; decoding
        // never fails, so only a collection error can surface above.
        let body = String::from_utf8_lossy(&collected.to_bytes()).into_owned();
        Ok(HttpResponse { status, body })
    }
}

/// Convert an elapsed request deadline into the established error shape.
fn request_timeout_error(context: Box<str>, timeout: Duration) -> VkError {
    VkError::RequestContext {
        context,
        source: Box::new(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("request timed out after {timeout:?}"),
        )),
    }
}

/// Boxed error alias for the low-level request-construction failures.
type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Build the POST request, serialising `payload` as a JSON body and applying
/// `headers` plus a `Content-Type: application/json` header.
///
/// Failures (an unparsable endpoint URL or payload serialisation) are returned
/// as boxed errors for the caller to wrap with the appropriate context.
fn build_request(
    endpoint: &Endpoint,
    headers: &HeaderMap,
    payload: &Value,
) -> Result<Request<Full<Bytes>>, BoxError> {
    let uri: Uri = endpoint.as_str().parse()?;
    let body = Bytes::from(serde_json::to_vec(payload)?);
    let mut request = Request::new(Full::new(body));
    *request.method_mut() = Method::POST;
    *request.uri_mut() = uri;
    let map = request.headers_mut();
    *map = headers.clone();
    map.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(request)
}

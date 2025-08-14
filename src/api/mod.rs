//! GraphQL client utilities and pagination helpers.
//!
//! This module wraps the GitHub GraphQL API, providing a `GraphQLClient`
//! with convenient functions for issuing queries. It also exposes the
//! `paginate` helper used throughout the binary for fetching all pages of
//! a cursor-based connection.

use log::warn;
use rand::Rng;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, USER_AGENT};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::env;
use tokio::time::{Duration, sleep};

use crate::VkError;
use crate::boxed::BoxedStr;

const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";

const BODY_SNIPPET_LEN: usize = 500;
const VALUE_SNIPPET_LEN: usize = 200;
const RETRY_ATTEMPTS: u32 = 5;
const RETRY_BASE_DELAY: Duration = Duration::from_millis(200);

fn snippet(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out = text.chars().take(max).collect::<String>();
        out.push_str("...");
        out
    }
}

#[derive(Debug, Deserialize)]
struct GraphQLResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: String,
}

fn handle_graphql_errors(errors: Vec<GraphQlError>) -> VkError {
    let msg = errors
        .into_iter()
        .map(|e| e.message)
        .collect::<Vec<_>>()
        .join(", ");
    VkError::ApiErrors(msg.boxed())
}

fn build_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, "vk".parse().expect("static string"));
    headers.insert(
        ACCEPT,
        "application/vnd.github+json"
            .parse()
            .expect("static string"),
    );
    if !token.is_empty() {
        headers.insert(
            AUTHORIZATION,
            format!("Bearer {token}").parse().expect("valid header"),
        );
    }
    headers
}

/// Client for communicating with the GitHub GraphQL API.
///
/// The client handles authentication headers and optional request
/// transcription for debugging.
pub struct GraphQLClient {
    client: reqwest::Client,
    headers: HeaderMap,
    endpoint: String,
    transcript: Option<std::sync::Mutex<std::io::BufWriter<std::fs::File>>>,
}

impl GraphQLClient {
    /// Create a client using the standard GitHub endpoint.
    ///
    /// The optional `transcript` path records each request and response
    /// for troubleshooting failed queries.
    ///
    /// # Errors
    ///
    /// Returns an [`std::io::Error`] if the transcript file cannot be opened.
    pub fn new(
        token: &str,
        transcript: Option<std::path::PathBuf>,
    ) -> Result<Self, std::io::Error> {
        let endpoint =
            env::var("GITHUB_GRAPHQL_URL").unwrap_or_else(|_| GITHUB_GRAPHQL_URL.to_string());
        Self::with_endpoint(token, &endpoint, transcript)
    }

    /// Create a client targeting a custom API endpoint.
    ///
    /// This is primarily used in tests to point the client at a mock
    /// server.
    ///
    /// # Errors
    ///
    /// Returns an [`std::io::Error`] if the transcript file cannot be opened.
    pub fn with_endpoint(
        token: &str,
        endpoint: &str,
        transcript: Option<std::path::PathBuf>,
    ) -> Result<Self, std::io::Error> {
        let transcript = match transcript {
            Some(p) => match std::fs::File::create(p) {
                Ok(file) => Some(std::sync::Mutex::new(std::io::BufWriter::new(file))),
                Err(e) => return Err(e),
            },
            None => None,
        };
        Ok(Self {
            client: reqwest::Client::new(),
            headers: build_headers(token),
            endpoint: endpoint.to_string(),
            transcript,
        })
    }

    fn should_retry(err: &VkError) -> bool {
        match err {
            VkError::RequestContext { .. } | VkError::Request(_) => true,
            VkError::BadResponse(msg) => msg.starts_with("Missing data in response"),
            _ => false,
        }
    }

    /// Execute a GraphQL query using this client.
    ///
    /// # Errors
    ///
    /// Returns a [`VkError`] if the request fails or the response cannot be
    /// deserialised.
    ///
    /// # Panics
    ///
    /// Panics if the retry base delay exceeds `u64` milliseconds.
    pub async fn run_query<V, T>(&self, query: &str, variables: V) -> Result<T, VkError>
    where
        V: serde::Serialize,
        T: DeserializeOwned,
    {
        let payload = json!({ "query": query, "variables": &variables });
        let ctx_box = serde_json::to_string(&payload).unwrap_or_default().boxed();
        let mut rng = rand::thread_rng();
        for attempt in 0..RETRY_ATTEMPTS {
            let result = async {
                let response = self
                    .client
                    .post(&self.endpoint)
                    .headers(self.headers.clone())
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| VkError::RequestContext {
                        context: ctx_box.clone(),
                        source: e.into(),
                    })?;
                let body = response.text().await.map_err(|e| VkError::RequestContext {
                    context: ctx_box.clone(),
                    source: e.into(),
                })?;
                if let Some(t) = &self.transcript {
                    use std::io::Write as _;
                    match t.lock() {
                        Ok(mut f) => {
                            if let Err(e) = writeln!(
                                f,
                                "{}",
                                serde_json::to_string(
                                    &json!({ "request": payload, "response": body })
                                )
                                .unwrap_or_default(),
                            ) {
                                warn!("failed to write transcript: {e}");
                            }
                        }
                        Err(_) => warn!("failed to lock transcript"),
                    }
                }
                let resp: GraphQLResponse<serde_json::Value> = serde_json::from_str(&body)
                    .map_err(|e| {
                        let snippet = snippet(&body, BODY_SNIPPET_LEN);
                        VkError::BadResponseSerde(
                            format!("{e} | response body snippet:{snippet}").boxed(),
                        )
                    })?;
                let resp_debug = format!("{resp:?}");
                if let Some(errs) = resp.errors {
                    return Err(handle_graphql_errors(errs));
                }
                let value = resp.data.ok_or_else(|| {
                    VkError::BadResponse(format!("Missing data in response: {resp_debug}").boxed())
                })?;
                match serde_path_to_error::deserialize::<_, T>(value.clone()) {
                    Ok(v) => Ok(v),
                    Err(e) => {
                        let snippet = snippet(
                            &serde_json::to_string_pretty(&value).unwrap_or_default(),
                            VALUE_SNIPPET_LEN,
                        );
                        let path = e.path().to_string();
                        let inner = e.into_inner();
                        Err(VkError::BadResponseSerde(
                            format!("{inner} at {path} | snippet: {snippet}").boxed(),
                        ))
                    }
                }
            }
            .await;
            match result {
                Ok(v) => return Ok(v),
                Err(e) if attempt + 1 < RETRY_ATTEMPTS && Self::should_retry(&e) => {
                    let base_ms =
                        u64::try_from(RETRY_BASE_DELAY.as_millis()).expect("delay fits in u64");
                    let backoff = RETRY_BASE_DELAY * 2u32.pow(attempt);
                    let jitter = Duration::from_millis(rng.gen_range(0..base_ms));
                    warn!("retrying GraphQL query after error: {e}");
                    sleep(backoff + jitter).await;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!("retry loop exhausted without returning");
    }
}

/// Retrieve all pages from a cursor-based connection.
///
/// The `fetch` closure is called repeatedly with the current cursor until the
/// [`PageInfo`] object indicates no further pages remain.
///
/// If the `fetch` closure yields an error, the function returns an [`Err`]
/// containing only that error. Any items fetched before the failure are
/// discarded and are not available in the error result.
///
/// # Examples
/// ```
/// use std::cell::Cell;
/// use vk::{api::paginate, PageInfo};
///
/// # tokio::runtime::Runtime::new().expect("runtime").block_on(async {
/// let calls = Cell::new(0);
/// let items = paginate(|_cursor| {
///     calls.set(calls.get() + 1);
///     let current = calls.get();
///     async move {
///         let (has_next_page, end_cursor) = if current == 1 {
///             (true, Some("next".to_string()))
///         } else {
///             (false, None)
///         };
///         Ok((vec![current], PageInfo { has_next_page, end_cursor }))
///     }
/// }).await.expect("pagination");
/// assert_eq!(items, vec![1, 2]);
/// assert_eq!(calls.get(), 2);
/// # });
/// ```
///
/// # Errors
///
/// Propagates any [`VkError`] returned by the `fetch` closure.
pub async fn paginate<T, F, Fut>(mut fetch: F) -> Result<Vec<T>, VkError>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<T>, crate::PageInfo), VkError>>,
{
    let mut items = Vec::new();
    let mut cursor = None;
    loop {
        let (mut page, info) = fetch(cursor.clone()).await?;
        items.append(&mut page);
        if !info.has_next_page {
            break;
        }
        cursor = info.end_cursor;
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    //! Tests for API utilities.

    use super::*;
    use crate::PageInfo;
    use std::{
        cell::RefCell,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use third_wheel::hyper::{
        Body, Request, Response, Server, StatusCode,
        service::{make_service_fn, service_fn},
    };
    use tokio::task::JoinHandle;

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
        let server = Server::bind(&"127.0.0.1:0".parse().expect("addr")).serve(svc);
        let addr = server.local_addr();
        let join = tokio::spawn(async move {
            let _ = server.await;
        });
        let client =
            GraphQLClient::with_endpoint("token", &format!("http://{addr}"), None).expect("client");
        TestClient { client, join }
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
}

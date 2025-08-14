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

/// Configuration for retrying failed GraphQL requests.
#[derive(Clone, Copy)]
pub struct RetryConfig {
    /// Total number of attempts including the initial request.
    pub attempts: u32,
    /// Base delay for the exponential backoff.
    pub base_delay: Duration,
    /// Jitter fraction of the backoff interval as a fixed-point value.
    ///
    /// The actual fraction is `jitter_factor / 128`; 128 means 100% jitter
    /// (up to the full backoff duration).
    pub jitter_factor: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            attempts: 5,
            base_delay: Duration::from_millis(200),
            jitter_factor: 128,
        }
    }
}

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
    retry: RetryConfig,
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
        Self::with_endpoint_retry(token, &endpoint, transcript, RetryConfig::default())
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
        Self::with_endpoint_retry(token, endpoint, transcript, RetryConfig::default())
    }

    /// Create a client targeting a custom API endpoint with custom retry settings.
    ///
    /// # Errors
    ///
    /// Returns an [`std::io::Error`] if the transcript file cannot be opened.
    pub fn with_endpoint_retry(
        token: &str,
        endpoint: &str,
        transcript: Option<std::path::PathBuf>,
        retry: RetryConfig,
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
            retry,
        })
    }

    fn should_retry(err: &VkError) -> bool {
        match err {
            VkError::RequestContext { .. } | VkError::Request(_) => true,
            VkError::BadResponse(msg) => msg.starts_with("Missing data in response"),
            _ => false,
        }
    }

    fn jittered_backoff(&self, attempt: u32, rng: &mut impl Rng) -> Duration {
        let backoff = self.retry.base_delay * 2u32.pow(attempt);
        let backoff_ms = u64::try_from(backoff.as_millis()).expect("delay fits in u64");
        let jitter_max = (backoff_ms * u64::from(self.retry.jitter_factor)) >> 7;
        let jitter = rng.gen_range(0..=jitter_max);
        Duration::from_millis(backoff_ms + jitter)
    }

    /// Execute an HTTP request and return the raw response body.
    ///
    /// # Errors
    ///
    /// Returns a [`VkError::RequestContext`] if the request fails or the
    /// response body cannot be read.
    async fn execute_single_request(
        &self,
        payload: &serde_json::Value,
        ctx: &str,
    ) -> Result<String, VkError> {
        let response = self
            .client
            .post(&self.endpoint)
            .headers(self.headers.clone())
            .json(payload)
            .send()
            .await
            .map_err(|e| VkError::RequestContext {
                context: ctx.to_owned().boxed(),
                source: e.into(),
            })?;
        response.text().await.map_err(|e| VkError::RequestContext {
            context: ctx.to_owned().boxed(),
            source: e.into(),
        })
    }

    /// Write the request and response to the transcript if enabled.
    fn log_transcript(&self, payload: &serde_json::Value, body: &str) {
        if let Some(t) = &self.transcript {
            use std::io::Write as _;
            match t.lock() {
                Ok(mut f) => {
                    if let Err(e) = writeln!(
                        f,
                        "{}",
                        serde_json::to_string(&json!({ "request": payload, "response": body }))
                            .expect("serialising GraphQL transcript"),
                    ) {
                        warn!("failed to write transcript: {e}");
                    }
                }
                Err(_) => warn!("failed to lock transcript"),
            }
        }
    }

    /// Parse a GraphQL response body into the desired type.
    ///
    /// # Errors
    ///
    /// Returns a [`VkError`] if the body cannot be deserialised or contains
    /// GraphQL errors.
    fn process_graphql_response<T>(body: &str) -> Result<T, VkError>
    where
        T: DeserializeOwned,
    {
        let resp: GraphQLResponse<serde_json::Value> = serde_json::from_str(body).map_err(|e| {
            let snippet = snippet(body, BODY_SNIPPET_LEN);
            VkError::BadResponseSerde(format!("{e} | response body snippet:{snippet}").boxed())
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
                    &serde_json::to_string_pretty(&value)
                        .expect("serialising JSON snippet for error"),
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

    /// Sleep for an exponential backoff with jitter.
    async fn perform_retry_delay(&self, attempt: u32, rng: &mut impl Rng) {
        sleep(self.jittered_backoff(attempt, rng)).await;
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
    /// Panics if the configured backoff exceeds `u64::MAX` milliseconds.
    pub async fn run_query<V, T>(&self, query: &str, variables: V) -> Result<T, VkError>
    where
        V: serde::Serialize,
        T: DeserializeOwned,
    {
        let payload = json!({ "query": query, "variables": &variables });
        let ctx = serde_json::to_string(&payload)
            .expect("serialising GraphQL request payload")
            .boxed();
        let mut rng = rand::thread_rng();
        let mut last_err = None;
        for attempt in 0..self.retry.attempts {
            let res = async {
                let body = self.execute_single_request(&payload, &ctx).await?;
                self.log_transcript(&payload, &body);
                Self::process_graphql_response::<T>(&body)
            }
            .await;
            match res {
                Ok(v) => return Ok(v),
                Err(e) => {
                    if attempt + 1 < self.retry.attempts && Self::should_retry(&e) {
                        warn!("retrying GraphQL query after error: {e}");
                        self.perform_retry_delay(attempt, &mut rng).await;
                        last_err = Some(e);
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| VkError::BadResponse("retry loop exhausted".boxed())))
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
mod tests;

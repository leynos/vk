//! GraphQL client utilities and pagination helpers.
//!
//! This module wraps the GitHub GraphQL API, providing a `GraphQLClient`
//! with convenient functions for issuing queries. It also exposes the
//! `paginate` helper used throughout the binary for fetching all pages of
//! a cursor-based connection.

use backon::{ExponentialBuilder, Retryable};
use log::warn;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, USER_AGENT};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use std::env;
use tokio::time::{Duration, sleep};

use crate::VkError;
use crate::boxed::BoxedStr;

/// A GraphQL query string with type safety.
#[derive(Debug, Clone)]
pub struct Query(String);

impl Query {
    pub fn new(query: impl Into<String>) -> Self {
        Self(query.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Query {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for Query {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A GitHub API authentication token.
#[derive(Debug, Clone)]
pub struct Token(String);

impl Token {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&str> for Token {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl AsRef<str> for Token {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A GitHub GraphQL API endpoint URL.
#[derive(Debug, Clone)]
pub struct Endpoint(String);

impl Endpoint {
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Endpoint {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for Endpoint {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl Default for Endpoint {
    fn default() -> Self {
        Self(GITHUB_GRAPHQL_URL.to_string())
    }
}

/// A pagination cursor for GraphQL connections.
#[derive(Debug, Clone)]
pub struct Cursor(String);

impl Cursor {
    pub fn new(cursor: impl Into<String>) -> Self {
        Self(cursor.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Cursor {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Cursor {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";

const BODY_SNIPPET_LEN: usize = 500;
const VALUE_SNIPPET_LEN: usize = 200;

/// Configuration for retrying failed GraphQL requests.
#[derive(Clone, Copy)]
pub struct RetryConfig {
    /// Total number of attempts including the initial request.
    pub attempts: usize,
    /// Base delay for the exponential backoff.
    pub base_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            attempts: 5,
            base_delay: Duration::from_millis(200),
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

fn build_headers(token: &Token) -> HeaderMap {
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
            format!("Bearer {}", token.as_str())
                .parse()
                .expect("valid header"),
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
    endpoint: Endpoint,
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
        token: impl Into<Token>,
        transcript: Option<std::path::PathBuf>,
    ) -> Result<Self, std::io::Error> {
        let token = token.into();
        let endpoint = env::var("GITHUB_GRAPHQL_URL")
            .map(Endpoint::new)
            .unwrap_or_default();
        Self::with_endpoint_retry(token, endpoint, transcript, RetryConfig::default())
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
        token: impl Into<Token>,
        endpoint: impl Into<Endpoint>,
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
        token: impl Into<Token>,
        endpoint: impl Into<Endpoint>,
        transcript: Option<std::path::PathBuf>,
        retry: RetryConfig,
    ) -> Result<Self, std::io::Error> {
        let token = token.into();
        let endpoint = endpoint.into();
        let transcript = match transcript {
            Some(p) => match std::fs::File::create(p) {
                Ok(file) => Some(std::sync::Mutex::new(std::io::BufWriter::new(file))),
                Err(e) => return Err(e),
            },
            None => None,
        };
        Ok(Self {
            client: reqwest::Client::new(),
            headers: build_headers(&token),
            endpoint,
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
            .post(self.endpoint.as_str())
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
    pub async fn run_query<V, T>(&self, query: impl Into<Query>, variables: V) -> Result<T, VkError>
    where
        V: serde::Serialize,
        T: DeserializeOwned,
    {
        let query = query.into();
        let payload = json!({ "query": query.as_ref(), "variables": &variables });
        let ctx = serde_json::to_string(&payload)
            .expect("serialising GraphQL request payload")
            .boxed();
        let builder = ExponentialBuilder::default()
            .with_min_delay(self.retry.base_delay)
            .with_max_times(self.retry.attempts)
            .with_jitter();
        (|| async {
            let body = self.execute_single_request(&payload, &ctx).await?;
            self.log_transcript(&payload, &body);
            Self::process_graphql_response::<T>(&body)
        })
        .retry(builder)
        .sleep(sleep)
        .when(|e: &VkError| Self::should_retry(e))
        .notify(|err: &VkError, dur| warn!("retrying GraphQL query after {dur:?}: {err}"))
        .await
    }

    /// Execute a GraphQL query and merge an optional cursor into the variables.
    ///
    /// This wraps [`run_query`], injecting the `cursor` field when provided so
    /// callers need only supply the base variables for paginated queries.
    ///
    /// # Errors
    ///
    /// Propagates any error from the underlying request.
    ///
    /// # Examples
    /// ```no_run
    /// use serde_json::{Map, Value, json};
    /// use vk::api::GraphQLClient;
    /// # async fn run(client: GraphQLClient) -> Result<(), vk::VkError> {
    /// let mut vars = Map::new();
    /// vars.insert("id".to_string(), json!(1));
    /// let data: Value = client.fetch_page("query", None, vars).await?;
    /// # Ok(())
    /// # }
    /// ```
    /// ```compile_fail
    /// use serde_json::json;
    /// use vk::api::GraphQLClient;
    /// # async fn run(client: GraphQLClient) {
    ///     // variables must be a JSON object
    ///     let _ = client
    ///         .fetch_page::<serde_json::Value>("query", None, json!(null))
    ///         .await;
    /// # }
    /// ```
    pub async fn fetch_page<T>(
        &self,
        query: impl Into<Query>,
        cursor: Option<Cursor>,
        mut variables: Map<String, Value>,
    ) -> Result<T, VkError>
    where
        T: DeserializeOwned,
    {
        let query = query.into();
        if let Some(c) = cursor {
            variables.insert("cursor".to_string(), Value::String(c.as_str().to_string()));
        }
        self.run_query(query, variables).await
    }

    /// Fetch and concatenate all pages from a cursor-based connection.
    ///
    /// `query` and `variables` define the base request. The `map` closure
    /// extracts the items and pagination info from each page's response.
    ///
    /// # Errors
    ///
    /// Propagates any [`VkError`] returned by the underlying request or mapper
    /// closure.
    pub async fn paginate_all<T, U, M>(
        &self,
        query: impl Into<Query>,
        variables: Map<String, Value>,
        mut start: Option<Cursor>,
        map: M,
    ) -> Result<Vec<U>, VkError>
    where
        T: DeserializeOwned,
        M: FnMut(T) -> Result<(Vec<U>, crate::PageInfo), VkError> + Clone,
    {
        let query = query.into();
        paginate(|cursor| {
            let vars = variables.clone();
            let mut mapper = map.clone();
            let current = cursor.map(Cursor::from).or_else(|| start.take());
            let q = query.clone();
            async move {
                let data = self.fetch_page::<T>(q, current, vars).await?;
                mapper(data)
            }
        })
        .await
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
        cursor = Some(info.end_cursor.ok_or_else(|| {
            VkError::BadResponse("hasNextPage=true but endCursor missing".boxed())
        })?);
    }
    Ok(items)
}

#[cfg(test)]
mod tests;

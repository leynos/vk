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
use std::{borrow::Cow, env};
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

const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";

const BODY_SNIPPET_LEN: usize = 500;
const VALUE_SNIPPET_LEN: usize = 200;

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    body: String,
}

/// Configuration for retrying failed GraphQL requests.
#[derive(Clone, Copy)]
pub struct RetryConfig {
    /// Total number of attempts including the initial request.
    pub attempts: usize,
    /// Base delay for the exponential backoff.
    pub base_delay: Duration,
    /// Whether to jitter the backoff delay.
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            attempts: 5,
            base_delay: Duration::from_millis(200),
            jitter: true,
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

fn operation_name(query: &str) -> Option<&str> {
    let trimmed = query.trim_start();
    for prefix in ["query", "mutation", "subscription"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            // Require a valid delimiter after the prefix to avoid false positives like "queryX".
            let first = rest.chars().next();
            let is_delim =
                matches!(first, Some(ch) if matches!(ch, '{' | '(' | ' ' | '\n' | '\t' | '\r'));
            if !is_delim {
                continue;
            }
            let rest = rest.trim_start();
            let name = rest
                .split(|c: char| c.is_whitespace() || c == '(' || c == '{')
                .next()
                .filter(|s| !s.is_empty());
            if let Some(name) = name {
                return Some(name);
            }
        }
    }
    None
}

#[derive(Debug, Deserialize)]
struct GraphQLResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQLError {
    message: String,
}

fn handle_graphql_errors(errors: Vec<GraphQLError>) -> VkError {
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

    fn is_transient_serde_error(status: u16, snippet: &str) -> bool {
        status >= 500 || status == 429 || snippet.trim_start().starts_with('<')
    }

    fn should_retry(err: &VkError) -> bool {
        match err {
            VkError::RequestContext { .. }
            | VkError::Request(_)
            | VkError::EmptyResponse { .. } => true,
            VkError::BadResponseSerde {
                status, snippet, ..
            } => Self::is_transient_serde_error(*status, snippet),
            _ => false,
        }
    }

    /// Execute an HTTP request and return the status code and body.
    ///
    /// # Errors
    ///
    /// Returns a [`VkError::RequestContext`] if the request fails or the
    /// response body cannot be read.
    async fn execute_single_request(
        &self,
        payload: &serde_json::Value,
        ctx: &str,
        operation: &str,
    ) -> Result<HttpResponse, VkError> {
        let response = self
            .client
            .post(self.endpoint.as_str())
            .headers(self.headers.clone())
            .json(payload)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| VkError::RequestContext {
                context: ctx.to_owned().boxed(),
                source: e.into(),
            })?;
        let status = response.status();
        let status_u16 = status.as_u16();
        let status_err = response.error_for_status_ref().err();
        let body = response.text().await.map_err(|e| VkError::RequestContext {
            context: format!("{ctx}; status {status_u16}").boxed(),
            source: e.into(),
        })?;
        if !(200..300).contains(&status_u16) {
            let resp = HttpResponse {
                status: status_u16,
                body: body.clone(),
            };
            self.log_transcript(payload, operation, &resp);
            let e = status_err.expect("status error for non-success status");
            return Err(VkError::RequestContext {
                context: format!(
                    "HTTP status {status_u16} | body snippet: {}",
                    snippet(&body, BODY_SNIPPET_LEN)
                )
                .boxed(),
                source: e.into(),
            });
        }
        Ok(HttpResponse {
            status: status_u16,
            body,
        })
    }

    /// Write the request and response to the transcript if enabled.
    fn log_transcript(&self, payload: &serde_json::Value, operation: &str, resp: &HttpResponse) {
        if let Some(t) = &self.transcript {
            use std::io::Write as _;
            match t.lock() {
                Ok(mut f) => {
                    if let Err(e) = writeln!(
                        f,
                        "{}",
                        serde_json::to_string(&json!({
                            "operation": operation,
                            "status": resp.status,
                            "request": payload,
                            "response": snippet(&resp.body, BODY_SNIPPET_LEN)
                        }))
                        .expect("serializing GraphQL transcript"),
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
    /// Returns a [`VkError`] if the body cannot be deserialized or contains
    /// GraphQL errors.
    fn process_graphql_response<T>(resp: &HttpResponse, operation: &str) -> Result<T, VkError>
    where
        T: DeserializeOwned,
    {
        let body = &resp.body;
        let status = resp.status;
        let resp: GraphQLResponse<serde_json::Value> = serde_json::from_str(body).map_err(|e| {
            let snippet = snippet(body, BODY_SNIPPET_LEN);
            VkError::BadResponseSerde {
                status,
                message: e.to_string().boxed(),
                snippet: snippet.boxed(),
            }
        })?;
        if let Some(errs) = resp.errors {
            return Err(handle_graphql_errors(errs));
        }
        let Some(value) = resp.data else {
            let body_snippet = snippet(body, BODY_SNIPPET_LEN);
            return Err(VkError::EmptyResponse {
                status,
                operation: operation.to_string().boxed(),
                snippet: body_snippet.boxed(),
            });
        };
        match serde_path_to_error::deserialize::<_, T>(value.clone()) {
            Ok(v) => Ok(v),
            Err(e) => {
                let snippet = snippet(
                    &serde_json::to_string_pretty(&value)
                        .expect("serializing JSON snippet for error"),
                    VALUE_SNIPPET_LEN,
                );
                let path = e.path().to_string();
                let inner = e.into_inner();
                Err(VkError::BadResponseSerde {
                    status,
                    message: format!("{inner} at {path}").boxed(),
                    snippet: snippet.boxed(),
                })
            }
        }
    }

    /// Execute a GraphQL query using this client.
    ///
    /// # Errors
    ///
    /// Returns a [`VkError`] if the request fails or the response cannot be
    /// deserialized.
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
        let op_name = operation_name(query.as_ref());
        let operation = op_name.map_or_else(|| snippet(query.as_ref(), 64), str::to_string);
        let mut payload = json!({ "query": query.as_ref(), "variables": &variables });
        if let (Some(_), Some(obj)) = (op_name, payload.as_object_mut()) {
            obj.insert("operationName".into(), json!(operation.clone()));
        }
        let payload_str =
            serde_json::to_string(&payload).expect("serializing GraphQL request payload");
        let ctx = format!("operation {operation}; {}", snippet(&payload_str, 1024)).boxed();
        let builder = {
            let b = ExponentialBuilder::default()
                .with_min_delay(self.retry.base_delay)
                .with_max_times(self.retry.attempts);
            if self.retry.jitter {
                b.with_jitter()
            } else {
                b
            }
        };
        (|| async {
            let resp = self
                .execute_single_request(&payload, &ctx, &operation)
                .await?;
            self.log_transcript(&payload, &operation, &resp);
            Self::process_graphql_response::<T>(&resp, &operation)
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
    /// callers need only supply the base variables for paginated queries. If the
    /// `variables` already contain a `cursor` key it will be overwritten.
    ///
    /// # Errors
    ///
    /// Returns [`VkError::BadResponse`] if `variables` serialize to a non-object
    /// value, or propagates any error from the underlying request.
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
    /// ```no_run
    /// use serde_json::json;
    /// use vk::api::GraphQLClient;
    /// # async fn run(client: GraphQLClient) {
    ///     let err = client
    ///         .fetch_page::<serde_json::Value, _>("query", None, json!(null))
    ///         .await;
    ///     assert!(err.is_err());
    /// # }
    /// ```
    pub async fn fetch_page<T, V>(
        &self,
        query: impl Into<Query>,
        cursor: Option<Cow<'_, str>>,
        variables: V,
    ) -> Result<T, VkError>
    where
        V: serde::Serialize,
        T: DeserializeOwned,
    {
        let query = query.into();
        let mut variables = serde_json::to_value(variables).map_err(|e| {
            VkError::BadResponse(format!("serialising fetch_page variables: {e}").boxed())
        })?;
        let obj = variables.as_object_mut().ok_or_else(|| {
            VkError::BadResponse("variables for fetch_page must be a JSON object".boxed())
        })?;
        if let Some(c) = cursor {
            obj.insert("cursor".into(), Value::String(c.into_owned()));
        }
        self.run_query(query, variables).await
    }

    /// Fetch and concatenate all pages from a cursor-based connection.
    ///
    /// `query` and `vars` define the base request. The `map` closure
    /// extracts the items and pagination info from each page's response.
    ///
    /// # Examples
    /// Borrowed and owned cursors both avoid allocations until needed.
    ///
    /// ```no_run
    /// use std::borrow::Cow;
    /// use serde_json::Map;
    /// use vk::{api::GraphQLClient, PageInfo, VkError};
    ///
    /// # async fn run(client: GraphQLClient) -> Result<(), VkError> {
    /// let vars = Map::new();
    /// client
    ///     .paginate_all::<(), _, serde_json::Value>(
    ///         "query",
    ///         vars.clone(),
    ///         Some(Cow::Borrowed("c1")),
    ///         |_page| Ok((Vec::new(), PageInfo::default())),
    ///     )
    ///     .await?;
    /// let owned = String::from("c2");
    /// client
    ///     .paginate_all::<(), _, serde_json::Value>(
    ///         "query",
    ///         vars,
    ///         Some(Cow::Owned(owned)),
    ///         |_page| Ok((Vec::new(), PageInfo::default())),
    ///     )
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Propagates any [`VkError`] returned by the underlying request or mapper
    /// closure.
    pub async fn paginate_all<Item, Mapper, Page>(
        &self,
        query: impl Into<Query>,
        vars: Map<String, Value>,
        start_cursor: Option<Cow<'_, str>>,
        mut map: Mapper,
    ) -> Result<Vec<Item>, VkError>
    where
        Mapper: FnMut(Page) -> Result<(Vec<Item>, crate::PageInfo), VkError>,
        Page: DeserializeOwned,
    {
        let query = query.into();
        let mut items = Vec::new();
        let mut cursor = start_cursor;
        loop {
            let vars = vars.clone();
            let data = self
                .fetch_page::<Page, _>(query.clone(), cursor.take(), vars)
                .await?;
            let (mut page, info) = map(data)?;
            items.append(&mut page);
            if let Some(next) = info.next_cursor()? {
                cursor = Some(Cow::Owned(next.to_string()));
            } else {
                break;
            }
        }
        Ok(items)
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
        if let Some(next) = info.next_cursor()? {
            cursor = Some(next.into());
        } else {
            break;
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests;

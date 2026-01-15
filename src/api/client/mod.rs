//! GraphQL client implementation and request orchestration.

mod helpers;
mod http;
mod pagination;
mod transcript;
mod types;

use backon::Retryable;
use reqwest::header::HeaderMap;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::borrow::Cow;
use tokio::time::sleep;
use tracing::warn;

use crate::VkError;
use crate::boxed::BoxedStr;
use vk::environment;

use self::helpers::{
    BODY_SNIPPET_LEN, VALUE_SNIPPET_LEN, build_headers, handle_graphql_errors, operation_name,
    payload_snippet, snippet,
};
use self::http::HttpResponse;
use self::types::GraphQLResponse;
use super::retry::{RetryConfig, build_retry_builder, should_retry};

pub use self::types::{Endpoint, Query, Token};

#[cfg(test)]
mod tests;

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
    /// Returns a [`VkError`] if the transcript file cannot be opened or the
    /// authorization header cannot be constructed.
    pub fn new(
        token: impl Into<Token>,
        transcript: Option<std::path::PathBuf>,
    ) -> Result<Self, VkError> {
        let token = token.into();
        let endpoint = environment::var("GITHUB_GRAPHQL_URL")
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
    /// Returns a [`VkError`] if the transcript file cannot be opened or the
    /// authorization header cannot be constructed.
    pub fn with_endpoint(
        token: impl Into<Token>,
        endpoint: impl Into<Endpoint>,
        transcript: Option<std::path::PathBuf>,
    ) -> Result<Self, VkError> {
        Self::with_endpoint_retry(token, endpoint, transcript, RetryConfig::default())
    }

    /// Create a client targeting a custom API endpoint with custom retry settings.
    ///
    /// # Errors
    ///
    /// Returns a [`VkError`] if the transcript file cannot be opened or the
    /// authorization header cannot be constructed.
    pub fn with_endpoint_retry(
        token: impl Into<Token>,
        endpoint: impl Into<Endpoint>,
        transcript: Option<std::path::PathBuf>,
        retry: RetryConfig,
    ) -> Result<Self, VkError> {
        let token = token.into();
        let endpoint = endpoint.into();
        let transcript = transcript
            .map(|p| {
                std::fs::File::create(p)
                    .map(|file| std::sync::Mutex::new(std::io::BufWriter::new(file)))
            })
            .transpose()
            .map_err(|e| VkError::Io(Box::new(e)))?;
        let headers = build_headers(&token)?;
        Ok(Self {
            client: reqwest::Client::new(),
            headers,
            endpoint,
            transcript,
            retry,
        })
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
        operation: &str,
    ) -> Result<HttpResponse, VkError> {
        let snip = payload_snippet(payload);
        let make_ctx = |status: Option<u16>| {
            let base = format!("operation {operation}; {snip}");
            match status {
                Some(s) => format!("{base}; status {s}"),
                None => base,
            }
            .boxed()
        };

        let response = self
            .client
            .post(self.endpoint.as_str())
            .headers(self.headers.clone())
            .json(payload)
            .timeout(self.retry.request_timeout)
            .send()
            .await
            .map_err(|e| VkError::RequestContext {
                context: make_ctx(None),
                source: e.into(),
            })?;
        let status = response.status();
        let status_u16 = status.as_u16();
        let status_err = response.error_for_status_ref().err();
        let body = response.text().await.map_err(|e| VkError::RequestContext {
            context: make_ctx(Some(status_u16)),
            source: e.into(),
        })?;
        let resp = HttpResponse {
            status: status_u16,
            body,
        };
        self.log_transcript(payload, operation, &resp);
        if !(200..300).contains(&status_u16) {
            let source: Box<dyn std::error::Error + Send + Sync> = match status_err {
                Some(e) => Box::new(e),
                None => Box::new(std::io::Error::other(format!(
                    "unexpected status {status_u16} without reqwest error"
                ))),
            };
            return Err(VkError::RequestContext {
                context: format!(
                    "HTTP status {status_u16} | body snippet: {}",
                    snippet(&resp.body, BODY_SNIPPET_LEN)
                )
                .boxed(),
                source,
            });
        }
        Ok(resp)
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
                let snippet = match serde_json::to_string_pretty(&value) {
                    Ok(json) => snippet(&json, VALUE_SNIPPET_LEN),
                    Err(e) => {
                        warn!("Failed to serialise error snippet: {e}");
                        "<failed to serialise error snippet>".to_string()
                    }
                };
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
        let builder = build_retry_builder(self.retry);
        (|| async {
            let resp = self.execute_single_request(&payload, &operation).await?;
            Self::process_graphql_response::<T>(&resp, &operation)
        })
        .retry(builder)
        .sleep(sleep)
        .when(should_retry)
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
}

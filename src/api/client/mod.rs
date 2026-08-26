//! GraphQL client implementation and request orchestration.

mod helpers;
mod http;
mod metrics;
mod pagination;
mod transcript;
mod transport;
mod types;

use backon::Retryable;
// `::http` disambiguates the extern crate from this module's own `http`
// submodule, which would otherwise shadow it.
use ::http::header::HeaderMap;
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
    snippet,
};
use self::http::HttpResponse;
use self::transport::{PostJsonRequest, Transport};
use self::types::GraphQLResponse;
use super::retry::{RetryConfig, build_retry_builder, should_retry};

pub use self::types::{Endpoint, Query, Token};

#[cfg(test)]
mod client_response_tests;
#[cfg(test)]
mod tests;

/// Client for communicating with the GitHub GraphQL API.
///
/// The client handles authentication headers and optional request
/// transcription for debugging.
pub struct GraphQLClient {
    /// Headers applied to every GraphQL request.
    headers: HeaderMap,
    /// GraphQL endpoint targeted by this client.
    endpoint: Endpoint,
    /// Optional writer for request and response transcripts.
    transcript: Option<std::sync::Mutex<std::io::BufWriter<std::fs::File>>>,
    /// Retry and timeout settings for requests.
    retry: RetryConfig,
    /// Pooled direct HTTP transport for GraphQL requests.
    transport: Transport,
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
            transport: Transport::new()?,
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
        let resp = self
            .transport
            .post_json(PostJsonRequest {
                endpoint: &self.endpoint,
                headers: &self.headers,
                payload,
                timeout: self.retry.request_timeout,
            })
            .await?;
        self.log_transcript(payload, operation, &resp);
        if !(200..300).contains(&resp.status) {
            // The transport surfaces every completed HTTP response, so a
            // non-2xx status is classified here. reqwest gave this a
            // status-specific source error via `error_for_status_ref`; an
            // `io::Error` carrying the same status reproduces the displayed
            // information without the reqwest dependency.
            let source: Box<dyn std::error::Error + Send + Sync> = Box::new(std::io::Error::other(
                format!("HTTP status {}", resp.status),
            ));
            return Err(VkError::RequestContext {
                context: format!(
                    "HTTP status {} | body snippet: {}",
                    resp.status,
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

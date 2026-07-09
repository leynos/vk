//! GraphQL client implementation and request orchestration.

mod helpers;
mod http;
mod pagination;
mod transcript;
mod transport;
mod types;

use backon::Retryable;
// `::http` disambiguates the extern crate from this module's own `http`
// submodule, which would otherwise shadow it.
use ::http::header::HeaderMap;
use serde::de::DeserializeOwned;
use tokio::time::sleep;
use tracing::warn;

use crate::VkError;
use crate::boxed::BoxedStr;
use vk::environment;

use self::helpers::{
    BODY_SNIPPET_LEN, VALUE_SNIPPET_LEN, build_headers, handle_graphql_errors, snippet,
};
use self::http::HttpResponse;
use self::transport::Transport;
use self::types::GraphQLResponse;
use super::retry::{RetryConfig, build_retry_builder, should_retry};

pub use self::types::{Endpoint, Token};

#[cfg(test)]
mod tests;

/// Client for communicating with the GitHub GraphQL API.
///
/// The client handles authentication headers and optional request
/// transcription for debugging.
pub struct GraphQLClient {
    transport: Transport,
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
            .post_json(
                &self.endpoint,
                &self.headers,
                payload,
                self.retry.request_timeout,
            )
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

    /// Send `payload` for `operation`, applying the shared retry loop and
    /// deserializing the successful response into `T`.
    ///
    /// This is the common core behind [`run_operation`](Self::run_operation)
    /// and [`run_operation_as`](Self::run_operation_as): both build the request
    /// envelope via `graphql_client` codegen and then delegate the POST,
    /// jittered-backoff retry, transcript logging, and `serde_path_to_error`
    /// diagnostics to this helper so the machinery is defined once.
    ///
    /// # Errors
    ///
    /// Returns a [`VkError`] if the request fails or the response cannot be
    /// deserialized.
    async fn run_payload<T>(
        &self,
        payload: &serde_json::Value,
        operation: &str,
    ) -> Result<T, VkError>
    where
        T: DeserializeOwned,
    {
        let builder = build_retry_builder(self.retry);
        (|| async {
            let resp = self.execute_single_request(payload, operation).await?;
            Self::process_graphql_response::<T>(&resp, operation)
        })
        .retry(builder)
        .sleep(sleep)
        .when(should_retry)
        .notify(|err: &VkError, dur| warn!("retrying GraphQL query after {dur:?}: {err}"))
        .await
    }

    /// Execute a `graphql_client` codegen'd GraphQL operation using this client.
    ///
    /// The operation's `QueryBody` serializes to the standard
    /// `{"query", "variables", "operationName"}` envelope, and the request is
    /// dispatched through the shared POST, retry, transcript, and
    /// deserialization core ([`run_payload`](Self::run_payload)), so behaviour
    /// (retries, error text, transcript shape) matches the retired string-based
    /// path exactly.
    ///
    /// # Errors
    ///
    /// Returns a [`VkError`] if the variables cannot be serialized, the request
    /// fails, or the response cannot be deserialized.
    ///
    /// # Examples
    /// ```no_run
    /// use graphql_client::GraphQLQuery;
    /// use vk::api::GraphQLClient;
    ///
    /// # async fn run<Q: GraphQLQuery>(
    /// #     client: GraphQLClient,
    /// #     variables: Q::Variables,
    /// # ) -> Result<Q::ResponseData, vk::VkError> {
    /// let data = client.run_operation::<Q>(variables).await?;
    /// # Ok(data)
    /// # }
    /// ```
    pub async fn run_operation<Q>(
        &self,
        variables: Q::Variables,
    ) -> Result<Q::ResponseData, VkError>
    where
        Q: graphql_client::GraphQLQuery,
    {
        self.run_operation_as::<Q, Q::ResponseData>(variables).await
    }

    /// Execute a codegen'd operation but deserialize the response into `T`
    /// rather than the generated `ResponseData`.
    ///
    /// This is the escape hatch used where the generated `ResponseData` would
    /// be stricter than the operation's documented behaviour or than the
    /// hand-written domain structs the tests and fixtures rely on (for example
    /// the review-thread listing, whose `isOutdated` field carries a
    /// client-side default, and the reviews listing, whose `state` is a GraphQL
    /// enum that the public `String` field preserves verbatim). The query is
    /// still built from `Q` — so `graphql_client` validates the selection
    /// against the vendored schema at compile time — while the wire body is
    /// decoded into `T` through the same `serde_path_to_error` machinery as
    /// [`run_operation`](Self::run_operation), keeping error text and retry
    /// behaviour identical.
    ///
    /// # Errors
    ///
    /// Returns a [`VkError`] if the variables cannot be serialized, the request
    /// fails, or the response cannot be deserialized into `T`.
    pub(crate) async fn run_operation_as<Q, T>(&self, variables: Q::Variables) -> Result<T, VkError>
    where
        Q: graphql_client::GraphQLQuery,
        T: DeserializeOwned,
    {
        let body = Q::build_query(variables);
        let operation = body.operation_name.to_string();
        let payload = serde_json::to_value(&body).map_err(|e| {
            VkError::BadResponse(format!("serialising {operation} variables: {e}").boxed())
        })?;
        self.run_payload::<T>(&payload, &operation).await
    }
}

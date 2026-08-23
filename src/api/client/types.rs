//! Types used by the GraphQL client.

use serde::Deserialize;

/// A GraphQL query string with type safety.
#[derive(Debug, Clone)]
pub struct Query(String);

impl Query {
    /// Create a query from a string-like value.
    pub fn new(query: impl Into<String>) -> Self {
        Self(query.into())
    }

    /// Return the query text.
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
    /// Create an authentication token from a string-like value.
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    /// Return the token text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Return whether the token contains no characters.
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
    /// Create an endpoint from a string-like URL.
    pub fn new(url: impl Into<String>) -> Self {
        Self(url.into())
    }

    /// Return the endpoint URL.
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

/// Default GitHub GraphQL API endpoint.
const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";

/// A decoded GraphQL response envelope.
#[derive(Debug, Deserialize)]
pub(super) struct GraphQLResponse<T> {
    /// Data returned by the operation, when present.
    pub(super) data: Option<T>,
    /// Errors reported by the GraphQL service, when present.
    pub(super) errors: Option<Vec<GraphQLError>>,
}

/// An error reported in a GraphQL response.
#[derive(Debug, Deserialize)]
pub(super) struct GraphQLError {
    /// Human-readable error message.
    pub(super) message: String,
}

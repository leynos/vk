//! Types used by the GraphQL client.

use serde::Deserialize;

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

#[derive(Debug, Deserialize)]
pub(super) struct GraphQLResponse<T> {
    pub(super) data: Option<T>,
    pub(super) errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GraphQLError {
    pub(super) message: String,
}

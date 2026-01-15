//! Helper utilities for GraphQL request handling.

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use serde_json::Value;
use tracing::warn;

use super::types::{GraphQLError, Token};
use crate::VkError;
use crate::boxed::BoxedStr;

/// Maximum number of characters to keep when logging response body snippets.
///
/// The limit keeps large payloads readable in error messages and transcripts.
pub(super) const BODY_SNIPPET_LEN: usize = 500;
/// Maximum number of characters to keep when logging request payload snippets.
///
/// Used for redacted GraphQL payloads to balance context and log size.
pub(super) const REQUEST_SNIPPET_LEN: usize = 1024;
/// Maximum number of characters to keep when logging individual value snippets.
///
/// The shorter length avoids overwhelming error messages with long values.
pub(super) const VALUE_SNIPPET_LEN: usize = 200;

/// Trim `text` to `max` characters, appending `...` when truncated.
///
/// Returns an empty string when `max` is zero.
pub(super) fn snippet(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out = text.chars().take(max).collect::<String>();
        out.push_str("...");
        out
    }
}

/// Recursively redact sensitive values from a JSON structure.
fn redact_sensitive(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if matches!(
                    k.to_ascii_lowercase().as_str(),
                    "token"
                        | "authorization"
                        | "password"
                        | "secret"
                        | "access_token"
                        | "refresh_token"
                        | "api_key"
                        | "apikey"
                        | "bearer"
                        | "auth"
                        | "credentials"
                        | "credential"
                        | "private_key"
                ) {
                    *v = Value::String("<redacted>".into());
                } else {
                    redact_sensitive(v);
                }
            }
        }
        Value::Array(arr) => arr.iter_mut().for_each(redact_sensitive),
        _ => {}
    }
}

/// Build a snippet of the redacted GraphQL payload.
///
/// Falls back to a placeholder when serialisation fails, logging the error.
pub(super) fn payload_snippet(payload: &Value) -> String {
    let mut redacted = payload.clone();
    redact_sensitive(&mut redacted);
    let json = match serde_json::to_string(&redacted) {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to serialise redacted payload: {e}");
            "<failed to serialise payload>".into()
        }
    };
    snippet(&json, REQUEST_SNIPPET_LEN)
}

/// Extract the operation name from a GraphQL query string.
///
/// Returns `None` when the input does not begin with a recognised operation
/// prefix or when no name is present.
pub(super) fn operation_name(query: &str) -> Option<&str> {
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

/// Convert GraphQL errors into a single `VkError` message.
pub(super) fn handle_graphql_errors(errors: Vec<GraphQLError>) -> VkError {
    let msg = errors
        .into_iter()
        .map(|e| e.message)
        .collect::<Vec<_>>()
        .join(", ");
    VkError::ApiErrors(msg.boxed())
}

/// Build standard GraphQL headers with an optional authorization token.
pub(super) fn build_headers(token: &Token) -> Result<HeaderMap, VkError> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("vk"));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    if !token.is_empty() {
        let value =
            format!("Bearer {}", token.as_str())
                .parse()
                .map_err(|e| VkError::RequestContext {
                    context: "parse Authorization header".to_string().boxed(),
                    source: Box::new(e),
                })?;
        headers.insert(AUTHORIZATION, value);
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::{
        GraphQLError, Token, build_headers, handle_graphql_errors, operation_name, payload_snippet,
        snippet,
    };
    use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    #[case("", 3, "")]
    #[case("abc", 0, "")]
    #[case("abc", 3, "abc")]
    #[case("abcd", 3, "abc...")]
    #[case("👍👍👍", 2, "👍👍...")]
    fn snippet_cases(#[case] text: &str, #[case] max: usize, #[case] expected: &str) {
        assert_eq!(snippet(text, max), expected);
    }

    #[rstest]
    #[case("query RetryOp { __typename }", Some("RetryOp"))]
    #[case("mutation UpdateThing { __typename }", Some("UpdateThing"))]
    #[case("subscription OnEvent { __typename }", Some("OnEvent"))]
    #[case("queryFoo { __typename }", None)]
    fn operation_name_cases(#[case] query: &str, #[case] expected: Option<&str>) {
        assert_eq!(operation_name(query), expected);
    }

    #[test]
    fn payload_snippet_redacts_sensitive_fields() {
        let payload = json!({
            "query": "query { viewer { login } }",
            "variables": {
                "token": "secret",
                "nested": {
                    "password": "p",
                    "api_key": "api-key-123"
                },
                "access_token": "access-789",
                "credentials": "creds-000",
                "private_key": "private-456"
            }
        });
        let snip = payload_snippet(&payload);
        assert!(!snip.contains("secret"));
        assert!(!snip.contains(":\"p\""));
        assert!(!snip.contains("api-key-123"));
        assert!(!snip.contains("access-789"));
        assert!(!snip.contains("creds-000"));
        assert!(!snip.contains("private-456"));
        assert!(snip.contains("<redacted>"));
    }

    #[test]
    fn handle_graphql_errors_joins_messages() {
        let err = handle_graphql_errors(vec![
            GraphQLError {
                message: "one".to_string(),
            },
            GraphQLError {
                message: "two".to_string(),
            },
        ]);
        assert_eq!(err.to_string(), "API errors: one, two");
    }

    #[test]
    fn build_headers_includes_base_headers_without_token() {
        let headers =
            build_headers(&Token::new("")).expect("failed to build headers without a token");
        assert!(headers.contains_key(USER_AGENT));
        assert!(headers.contains_key(ACCEPT));
        assert!(!headers.contains_key(AUTHORIZATION));
    }

    #[test]
    fn build_headers_adds_authorization_with_token() {
        let headers =
            build_headers(&Token::new("token")).expect("failed to build headers with a token");
        let auth = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .expect("authorization header missing or invalid for token");
        assert_eq!(auth, "Bearer token");
    }
}

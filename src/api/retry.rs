//! Retry configuration and policy helpers for GraphQL requests.

use crate::VkError;
use backon::ExponentialBuilder;
use tokio::time::Duration;

/// Configuration for retrying failed GraphQL requests.
#[derive(Clone, Copy, Debug)]
pub struct RetryConfig {
    /// Total number of attempts including the initial request.
    pub attempts: usize,
    /// Base delay for the exponential backoff.
    pub base_delay: Duration,
    /// Request timeout applied to each HTTP call.
    pub request_timeout: Duration,
    /// Whether to jitter the backoff delay.
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            attempts: 5,
            base_delay: Duration::from_millis(200),
            request_timeout: Duration::from_secs(30),
            jitter: true,
        }
    }
}

/// Build an exponential backoff retry builder from the given configuration.
///
/// The builder uses the configured attempt count and base delay, and applies
/// jitter when enabled.
///
/// # Examples
/// ```ignore
/// use vk::api::RetryConfig;
/// use vk::api::retry::build_retry_builder;
///
/// let config = RetryConfig {
///     attempts: 3,
///     ..RetryConfig::default()
/// };
/// let _builder = build_retry_builder(config);
/// ```
pub fn build_retry_builder(config: RetryConfig) -> ExponentialBuilder {
    let builder = ExponentialBuilder::default()
        .with_min_delay(config.base_delay)
        .with_max_times(config.attempts);
    if config.jitter {
        builder.with_jitter()
    } else {
        builder
    }
}

/// Decide whether a request should be retried based on the error.
///
/// Network errors, empty responses, and transient deserialisation failures
/// are treated as retryable.
///
/// # Examples
/// ```ignore
/// use vk::VkError;
/// use vk::api::retry::should_retry;
///
/// let err = VkError::RequestContext {
///     context: "ctx".into(),
///     source: Box::new(std::io::Error::other("boom")),
/// };
/// assert!(should_retry(&err));
/// ```
pub fn should_retry(err: &VkError) -> bool {
    match err {
        VkError::RequestContext { .. } | VkError::Request(_) | VkError::EmptyResponse { .. } => {
            true
        }
        VkError::BadResponseSerde {
            status, snippet, ..
        } => is_transient_serde_error(*status, snippet),
        _ => false,
    }
}

fn is_transient_serde_error(status: u16, snippet: &str) -> bool {
    status >= 500 || status == 429 || snippet.trim_start().starts_with('<')
}

#[cfg(test)]
mod tests {
    use super::{is_transient_serde_error, should_retry};
    use crate::VkError;

    #[test]
    fn should_retry_request_and_empty_response() {
        let err = VkError::RequestContext {
            context: "ctx".into(),
            source: Box::new(std::io::Error::other("boom")),
        };
        assert!(should_retry(&err));

        let err = VkError::EmptyResponse {
            status: 500,
            operation: "op".into(),
            snippet: "body".into(),
        };
        assert!(should_retry(&err));
    }

    #[test]
    fn should_retry_handles_bad_response_serde() {
        let err = VkError::BadResponseSerde {
            status: 429,
            message: "bad".into(),
            snippet: "<html>oops</html>".into(),
        };
        assert!(should_retry(&err));

        let err = VkError::BadResponseSerde {
            status: 400,
            message: "bad".into(),
            snippet: "{\"error\":\"nope\"}".into(),
        };
        assert!(!should_retry(&err));
    }

    #[test]
    fn is_transient_serde_error_detects_html_or_status() {
        assert!(is_transient_serde_error(500, "{}"));
        assert!(is_transient_serde_error(429, "{}"));
        assert!(is_transient_serde_error(400, "<html>"));
        assert!(!is_transient_serde_error(400, "{\"error\":\"nope\"}"));
    }
}

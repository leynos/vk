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
/// The `request_timeout` field is handled by the HTTP client and is not used
/// to configure the backoff policy.
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

/// Determines whether a `VkError` is transient and should be retried.
/// Returns `true` for network errors (`VkError::RequestContext`,
/// `VkError::Request`), empty responses (`VkError::EmptyResponse`), and for
/// `VkError::BadResponseSerde` errors only when
/// `is_transient_serde_error(status, snippet)` returns true (indicating
/// server-side or rate-limit failures). All other `VkError` variants return
/// `false`.
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

/// Determine whether a deserialization error looks transient.
///
/// HTML bodies or 5xx/429 responses are treated as retryable.
fn is_transient_serde_error(status: u16, snippet: &str) -> bool {
    status >= 500 || status == 429 || snippet.trim_start().starts_with('<')
}

#[cfg(test)]
mod tests {
    use super::{is_transient_serde_error, should_retry};
    use crate::VkError;
    use rstest::rstest;

    #[rstest]
    #[case(
        VkError::RequestContext {
            context: "ctx".into(),
            source: Box::new(std::io::Error::other("boom")),
        },
        true
    )]
    #[case(
        VkError::EmptyResponse {
            status: 500,
            operation: "op".into(),
            snippet: "body".into(),
        },
        true
    )]
    #[case(
        VkError::BadResponseSerde {
            status: 429,
            message: "bad".into(),
            snippet: "<html>oops</html>".into(),
        },
        true
    )]
    #[case(
        VkError::BadResponseSerde {
            status: 400,
            message: "bad".into(),
            snippet: "{\"error\":\"nope\"}".into(),
        },
        false
    )]
    #[case(VkError::ApiErrors("boom".into()), false)]
    fn should_retry_cases(#[case] err: VkError, #[case] expected: bool) {
        assert_eq!(should_retry(&err), expected);
    }

    #[rstest]
    #[case(500, "{}", true)]
    #[case(429, "{}", true)]
    #[case(400, "<html>", true)]
    #[case(400, "{\"error\":\"nope\"}", false)]
    fn is_transient_serde_error_cases(
        #[case] status: u16,
        #[case] snippet: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(is_transient_serde_error(status, snippet), expected);
    }
}

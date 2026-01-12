//! Retry configuration and policy helpers for GraphQL requests.

use crate::VkError;
use backon::ExponentialBuilder;
use tokio::time::Duration;

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

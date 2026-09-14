//! Transcript logging for GraphQL requests.

use super::GraphQLClient;
use super::HttpResponse;
use super::helpers::{BODY_SNIPPET_LEN, snippet};
use serde_json::json;

impl GraphQLClient {
    /// Write the request and response to the transcript if enabled.
    pub(super) async fn log_transcript(
        &self,
        payload: &serde_json::Value,
        operation: &str,
        resp: &HttpResponse,
    ) -> Result<(), crate::VkError> {
        let Some(transcript) = &self.transcript else {
            return Ok(());
        };
        let entry = serde_json::to_string(&json!({
            "operation": operation,
            "status": resp.status,
            "request": payload,
            "response": snippet(&resp.body, BODY_SNIPPET_LEN)
        }))
        .map_err(|error| {
            crate::VkError::BadResponse(format!("serializing transcript: {error}").into())
        })?;
        let transcript = std::sync::Arc::clone(transcript);
        tokio::task::spawn_blocking(move || {
            use std::io::Write as _;

            let mut writer = transcript
                .lock()
                .map_err(|error| std::io::Error::other(format!("locking transcript: {error}")))?;
            writeln!(writer, "{entry}")?;
            writer.flush()
        })
        .await
        .map_err(|error| crate::VkError::Io(Box::new(std::io::Error::other(error))))?
        .map_err(|error| crate::VkError::Io(Box::new(error)))
    }
}

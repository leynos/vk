//! Transcript logging for GraphQL requests.

use super::GraphQLClient;
use super::HttpResponse;
use super::helpers::{BODY_SNIPPET_LEN, snippet};
use super::metrics::{TranscriptFailure, record_transcript_failure};
use serde_json::json;
use tracing::warn;

/// One failure encountered while recording a transcript entry.
enum TranscriptWriteError {
    /// Acquiring the writer lock failed.
    Lock(String),
    /// Writing the entry failed.
    Write(std::io::Error),
    /// Flushing the writer failed.
    Flush(std::io::Error),
}

impl TranscriptWriteError {
    /// Return the fixed failure stage for metrics and structured diagnostics.
    const fn failure(&self) -> TranscriptFailure {
        match self {
            Self::Lock(_) => TranscriptFailure::Lock,
            Self::Write(_) => TranscriptFailure::Write,
            Self::Flush(_) => TranscriptFailure::Flush,
        }
    }
}

impl std::fmt::Display for TranscriptWriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lock(error) => write!(formatter, "locking transcript: {error}"),
            Self::Write(error) => write!(formatter, "writing transcript: {error}"),
            Self::Flush(error) => write!(formatter, "flushing transcript: {error}"),
        }
    }
}

impl GraphQLClient {
    /// Write the request and response to the transcript if enabled.
    pub(super) async fn log_transcript(
        &self,
        payload: &serde_json::Value,
        operation: &str,
        resp: &HttpResponse,
    ) {
        let Some(transcript) = &self.transcript else {
            return;
        };
        let entry = serde_json::to_string(&json!({
            "operation": operation,
            "status": resp.status,
            "request": payload,
            "response": snippet(&resp.body, BODY_SNIPPET_LEN)
        }))
        .map_err(|error| {
            record_transcript_failure(TranscriptFailure::Serialization);
            warn!(%operation, stage = "serialization", %error, "failed to record GraphQL transcript");
        });
        let Ok(entry) = entry else {
            return;
        };
        let transcript = std::sync::Arc::clone(transcript);
        let result = tokio::task::spawn_blocking(move || {
            use std::io::Write as _;

            let mut writer = transcript
                .lock()
                .map_err(|error| TranscriptWriteError::Lock(error.to_string()))?;
            writeln!(writer, "{entry}").map_err(TranscriptWriteError::Write)?;
            writer.flush().map_err(TranscriptWriteError::Flush)
        })
        .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                record_transcript_failure(error.failure());
                warn!(%operation, stage = error.failure().label(), %error, "failed to record GraphQL transcript");
            }
            Err(error) => {
                record_transcript_failure(TranscriptFailure::Write);
                warn!(%operation, stage = "write", %error, "failed to record GraphQL transcript");
            }
        }
    }
}

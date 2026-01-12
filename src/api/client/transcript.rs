//! Transcript logging for GraphQL requests.

use serde_json::json;
use tracing::warn;

use super::GraphQLClient;
use super::HttpResponse;
use super::helpers::{BODY_SNIPPET_LEN, snippet};

impl GraphQLClient {
    /// Write the request and response to the transcript if enabled.
    pub(super) fn log_transcript(
        &self,
        payload: &serde_json::Value,
        operation: &str,
        resp: &HttpResponse,
    ) {
        if let Some(t) = &self.transcript {
            use std::io::Write as _;
            match t.lock() {
                Ok(mut f) => {
                    if let Err(e) = writeln!(
                        f,
                        "{}",
                        serde_json::to_string(&json!({
                            "operation": operation,
                            "status": resp.status,
                            "request": payload,
                            "response": snippet(&resp.body, BODY_SNIPPET_LEN)
                        }))
                        .expect("serializing GraphQL transcript"),
                    ) {
                        warn!("failed to write transcript for op={operation}: {e}");
                        return;
                    }
                    if let Err(e) = f.flush() {
                        warn!("failed to flush transcript for op={operation}: {e}");
                    }
                }
                Err(e) => {
                    warn!("failed to lock transcript for op={operation}: {e}");
                }
            }
        }
    }
}

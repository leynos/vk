//! Bounded metrics emitted by the REST review-comment reply boundary.
//!
//! This private module records one observation per non-empty reply attempt.
//! Its labels deliberately exclude repository, pull-request, comment, route,
//! and error values so an application recorder has a fixed cardinality.

use super::rest_invariants::{ReplyStatus, status_class};
use http::StatusCode;
use metrics::{Unit, counter, histogram};
use std::time::Instant;

const REQUEST_COUNT: &str = "vk.resolve.rest_reply.requests.total";
const REQUEST_DURATION: &str = "vk.resolve.rest_reply.duration.seconds";
const TIMEOUT_COUNT: &str = "vk.resolve.rest_reply.timeouts.total";

/// Result of one request attempt, expressed with bounded labels.
pub(super) enum ReplyAttemptOutcome {
    /// The REST server returned a response.
    Response {
        status: StatusCode,
        result: ReplyStatus,
    },
    /// The total request deadline expired.
    Timeout,
    /// The REST transport failed before receiving a response.
    TransportFailure,
}

impl ReplyAttemptOutcome {
    fn labels(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Response {
                status,
                result: ReplyStatus::Success,
            } => ("success", status_class(status), "none"),
            Self::Response {
                status,
                result: ReplyStatus::NotFound,
            } => ("not_found", status_class(status), "none"),
            Self::Response {
                status,
                result: ReplyStatus::Failure,
            } => ("failure", status_class(status), "http_status"),
            Self::Timeout => ("failure", "none", "timeout"),
            Self::TransportFailure => ("failure", "none", "transport"),
        }
    }
}

/// Records one REST reply attempt and its elapsed duration.
pub(super) fn record_reply_attempt(started_at: Instant, outcome: ReplyAttemptOutcome) {
    let (outcome, status_class, failure_category) = outcome.labels();
    counter!(
        description: "Count REST review-comment reply attempts by bounded outcome.",
        REQUEST_COUNT,
        "outcome" => outcome,
        "status_class" => status_class,
        "failure_category" => failure_category,
    )
    .increment(1);
    histogram!(
        description: "Measure elapsed seconds for REST review-comment reply attempts.",
        unit: Unit::Seconds,
        REQUEST_DURATION,
        "outcome" => outcome,
        "status_class" => status_class,
        "failure_category" => failure_category,
    )
    .record(started_at.elapsed().as_secs_f64());
    if matches!(failure_category, "timeout") {
        counter!(
            description: "Count REST review-comment replies whose total deadline expired.",
            TIMEOUT_COUNT,
        )
        .increment(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics::{Key, Label, with_local_recorder};
    use metrics_util::{
        MetricKind,
        debugging::{DebugValue, DebuggingRecorder},
    };

    #[test]
    fn records_bounded_response_and_timeout_metrics() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        with_local_recorder(&recorder, || {
            record_reply_attempt(
                Instant::now(),
                ReplyAttemptOutcome::Response {
                    status: StatusCode::NOT_FOUND,
                    result: ReplyStatus::NotFound,
                },
            );
            record_reply_attempt(Instant::now(), ReplyAttemptOutcome::Timeout);
        });

        let snapshot = snapshotter.snapshot().into_vec();
        let reply_labels = vec![
            Label::new("outcome", "not_found"),
            Label::new("status_class", "4xx"),
            Label::new("failure_category", "none"),
        ];
        let timeout_labels = vec![
            Label::new("outcome", "failure"),
            Label::new("status_class", "none"),
            Label::new("failure_category", "timeout"),
        ];
        assert!(snapshot.iter().any(|(key, _, _, value)| {
            *key == metrics_util::CompositeKey::new(
                MetricKind::Counter,
                Key::from_parts(REQUEST_COUNT, reply_labels.clone()),
            ) && *value == DebugValue::Counter(1)
        }));
        assert!(snapshot.iter().any(|(key, unit, _, value)| {
            *key == metrics_util::CompositeKey::new(
                MetricKind::Histogram,
                Key::from_parts(REQUEST_DURATION, timeout_labels.clone()),
            ) && *unit == Some(Unit::Seconds)
                && matches!(value, DebugValue::Histogram(samples) if samples.len() == 1)
        }));
        assert!(snapshot.iter().any(|(key, _, _, value)| {
            *key == metrics_util::CompositeKey::new(
                MetricKind::Counter,
                Key::from_name(TIMEOUT_COUNT),
            ) && *value == DebugValue::Counter(1)
        }));
    }
}

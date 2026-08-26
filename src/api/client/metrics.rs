//! Bounded metrics for the GraphQL HTTP transport.
//!
//! The transport owns these observations because it alone distinguishes HTTP,
//! timeout, transport, body-read, and body-limit outcomes. Labels are fixed
//! classifications rather than request data, keeping tokens, payloads, URLs,
//! operation names, and response content out of metric cardinality.

use metrics::{Unit, counter, histogram};
use std::time::Instant;
use tracing::Span;

/// Metric counting GraphQL transport attempts.
const REQUEST_COUNT: &str = "vk.api.graphql_transport.requests.total";
/// Metric recording GraphQL transport attempt duration in seconds.
const REQUEST_DURATION: &str = "vk.api.graphql_transport.duration.seconds";

/// Outcome of one GraphQL transport attempt expressed with bounded labels.
#[derive(Clone, Copy, Debug)]
pub(super) enum GraphQLAttemptOutcome {
    /// The HTTP exchange completed with a response status.
    Response {
        /// HTTP status returned by the server.
        status: u16,
    },
    /// The whole-exchange deadline expired.
    Timeout {
        /// Status received before the deadline elapsed, if response headers arrived.
        status: Option<u16>,
    },
    /// Request construction or the transport failed before a response arrived.
    TransportFailure,
    /// Reading a response body failed after headers arrived.
    BodyReadFailure {
        /// HTTP status returned before the body read failed.
        status: u16,
    },
    /// The response body exceeded the configured byte limit.
    BodyLimitFailure {
        /// HTTP status returned before the limit was exceeded.
        status: u16,
    },
}

impl GraphQLAttemptOutcome {
    /// Return the fixed metric labels for this outcome.
    fn labels(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Response { status } if (200..300).contains(&status) => {
                ("success", status_class(status), "none")
            }
            Self::Response { status } => ("failure", status_class(status), "http_status"),
            Self::Timeout { status } => ("failure", status.map_or("none", status_class), "timeout"),
            Self::TransportFailure => ("failure", "none", "transport"),
            Self::BodyReadFailure { status } => ("failure", status_class(status), "body_read"),
            Self::BodyLimitFailure { status } => ("failure", status_class(status), "body_limit"),
        }
    }
}

/// Classify every possible status value into a fixed label set.
fn status_class(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "other",
    }
}

/// Record the current attempt with its duration and bounded labels.
pub(super) fn record_transport_attempt(started_at: Instant, outcome: GraphQLAttemptOutcome) {
    let (outcome, status_class, failure_category) = outcome.labels();
    Span::current().record("outcome", outcome);
    Span::current().record("status_class", status_class);
    Span::current().record("failure_category", failure_category);
    counter!(
        description: "Count GraphQL HTTP transport attempts by bounded outcome.",
        REQUEST_COUNT,
        "outcome" => outcome,
        "status_class" => status_class,
        "failure_category" => failure_category,
    )
    .increment(1);
    histogram!(
        description: "Measure elapsed seconds for GraphQL HTTP transport attempts.",
        unit: Unit::Seconds,
        REQUEST_DURATION,
        "outcome" => outcome,
        "status_class" => status_class,
        "failure_category" => failure_category,
    )
    .record(started_at.elapsed().as_secs_f64());
}

#[cfg(test)]
mod tests {
    //! Tests for GraphQL transport metrics.

    use super::*;
    use metrics::{Key, Label, with_local_recorder};
    use metrics_util::{
        MetricKind,
        debugging::{DebugValue, DebuggingRecorder},
    };
    use proptest::prelude::*;
    use rstest::rstest;

    #[rstest]
    #[case(GraphQLAttemptOutcome::Response { status: 200 })]
    #[case(GraphQLAttemptOutcome::Response { status: 503 })]
    #[case(GraphQLAttemptOutcome::Timeout { status: None })]
    #[case(GraphQLAttemptOutcome::Timeout { status: Some(503) })]
    #[case(GraphQLAttemptOutcome::TransportFailure)]
    #[case(GraphQLAttemptOutcome::BodyReadFailure { status: 503 })]
    #[case(GraphQLAttemptOutcome::BodyLimitFailure { status: 503 })]
    fn records_bounded_metrics_for_each_transport_outcome(#[case] attempt: GraphQLAttemptOutcome) {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let (outcome, status_class, failure_category) = attempt.labels();
        with_local_recorder(&recorder, || {
            record_transport_attempt(Instant::now(), attempt);
        });

        let snapshot = snapshotter.snapshot().into_vec();
        let labels = vec![
            Label::new("outcome", outcome),
            Label::new("status_class", status_class),
            Label::new("failure_category", failure_category),
        ];
        assert!(snapshot.iter().any(|(key, _, _, value)| {
            *key == metrics_util::CompositeKey::new(
                MetricKind::Counter,
                Key::from_parts(REQUEST_COUNT, labels.clone()),
            ) && *value == DebugValue::Counter(1)
        }));
        assert!(snapshot.iter().any(|(key, unit, _, value)| {
            *key == metrics_util::CompositeKey::new(
                MetricKind::Histogram,
                Key::from_parts(REQUEST_DURATION, labels.clone()),
            ) && *unit == Some(Unit::Seconds)
                && matches!(value, DebugValue::Histogram(samples) if samples.len() == 1)
        }));
    }

    proptest! {
        #[test]
        fn status_and_outcome_labels_stay_within_the_documented_sets(status in any::<u16>()) {
            let outcomes = [
                GraphQLAttemptOutcome::Response { status },
                GraphQLAttemptOutcome::Timeout { status: None },
                GraphQLAttemptOutcome::Timeout {
                    status: Some(status),
                },
                GraphQLAttemptOutcome::TransportFailure,
                GraphQLAttemptOutcome::BodyReadFailure { status },
                GraphQLAttemptOutcome::BodyLimitFailure { status },
            ];
            for outcome in outcomes {
                let (outcome, status_class, failure_category) = outcome.labels();
                prop_assert!(matches!(outcome, "success" | "failure"));
                prop_assert!(matches!(status_class, "none" | "1xx" | "2xx" | "3xx" | "4xx" | "5xx" | "other"));
                prop_assert!(matches!(failure_category, "none" | "http_status" | "timeout" | "transport" | "body_read" | "body_limit"));
            }
        }
    }
}

//! Bounded metrics emitted by the REST review-comment reply boundary.
//!
//! This private module records one observation per non-empty reply attempt.
//! Its labels deliberately exclude repository, pull-request, comment, route,
//! and error values so an application recorder has a fixed cardinality.

use super::rest_invariants::{ReplyStatus, status_class};
use http::StatusCode;
use metrics::{Unit, counter, histogram};
use std::time::Instant;

/// Metric counting REST reply attempts.
const REQUEST_COUNT: &str = "vk.resolve.rest_reply.requests.total";
/// Metric recording REST reply duration in seconds.
const REQUEST_DURATION: &str = "vk.resolve.rest_reply.duration.seconds";
/// Metric counting REST reply attempts that exceed the total deadline.
const TIMEOUT_COUNT: &str = "vk.resolve.rest_reply.timeouts.total";

/// Result of one request attempt, expressed with bounded labels.
pub(super) enum ReplyAttemptOutcome {
    /// The REST server returned a response.
    Response {
        /// HTTP status returned by the REST server.
        status: StatusCode,
        /// Classification applied to the HTTP status.
        result: ReplyStatus,
    },
    /// The total request deadline expired.
    Timeout,
    /// The REST transport failed before receiving a response.
    TransportFailure,
}

impl ReplyAttemptOutcome {
    /// Return the bounded metric labels for this outcome.
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
    //! Tests for REST reply metrics.

    use super::*;
    use crate::{
        ref_parser::RepoInfo,
        resolve::{
            CommentRef,
            rest::{RestClient, post_reply},
        },
    };
    use bytes::Bytes;
    use http_body_util::Full;
    use hyper::{Response, service::service_fn};
    use hyper_util::{
        rt::{TokioExecutor, TokioIo},
        server::conn::auto,
    };
    use metrics::{Key, Label, with_local_recorder};
    use metrics_util::{
        MetricKind,
        debugging::{DebugValue, DebuggingRecorder},
    };
    use rstest::rstest;
    use std::{convert::Infallible, time::Duration};
    use tokio::{net::TcpListener, runtime::Builder};

    #[derive(Copy, Clone)]
    enum ReplyAttempt {
        Success,
        NotFound,
        Failure,
        Timeout,
        TransportFailure,
    }

    impl ReplyAttempt {
        fn expected_labels(self) -> (&'static str, &'static str, &'static str) {
            match self {
                Self::Success => ("success", "2xx", "none"),
                Self::NotFound => ("not_found", "4xx", "none"),
                Self::Failure => ("failure", "5xx", "http_status"),
                Self::Timeout => ("failure", "none", "timeout"),
                Self::TransportFailure => ("failure", "none", "transport"),
            }
        }

        fn should_succeed(self) -> bool {
            matches!(self, Self::Success | Self::NotFound)
        }

        fn is_timeout(self) -> bool {
            matches!(self, Self::Timeout)
        }

        async fn serve(self, listener: TcpListener) {
            let (stream, _) = listener.accept().await.expect("accept REST request");
            match self {
                Self::Success => serve_response(stream, StatusCode::OK).await,
                Self::NotFound => serve_response(stream, StatusCode::NOT_FOUND).await,
                Self::Failure => serve_response(stream, StatusCode::INTERNAL_SERVER_ERROR).await,
                Self::Timeout => {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    drop(stream);
                }
                Self::TransportFailure => drop(stream),
            }
        }
    }

    async fn serve_response(stream: tokio::net::TcpStream, status: StatusCode) {
        let service = service_fn(move |_| async move {
            Ok::<_, Infallible>(
                Response::builder()
                    .status(status)
                    .body(Full::new(Bytes::new()))
                    .expect("build REST response"),
            )
        });
        let _ = auto::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(stream), service)
            .await;
    }

    #[rstest]
    #[case::success(ReplyAttempt::Success)]
    #[case::not_found(ReplyAttempt::NotFound)]
    #[case::failure(ReplyAttempt::Failure)]
    #[case::timeout(ReplyAttempt::Timeout)]
    #[case::transport_failure(ReplyAttempt::TransportFailure)]
    fn records_bounded_metrics_for_each_reply_outcome(#[case] attempt: ReplyAttempt) {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        with_local_recorder(&recorder, || {
            let runtime = Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build test runtime");
            let result = runtime.block_on(async {
                let listener = TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("bind REST stub");
                let address = listener.local_addr().expect("get REST stub address");
                let server = tokio::spawn(attempt.serve(listener));
                let rest = RestClient::new(
                    "token",
                    Some(&format!("http://{address}")),
                    Duration::from_millis(20),
                    Duration::from_secs(1),
                )
                .expect("build REST client");
                let repo = RepoInfo {
                    owner: "octocat".into(),
                    name: "hello-world".into(),
                };
                let reference = CommentRef {
                    repo: &repo,
                    pull_number: 1,
                    comment_id: 42,
                };
                let result = post_reply(&rest, reference, "reply").await;
                server.abort();
                let _ = server.await;
                result
            });
            assert_eq!(result.is_ok(), attempt.should_succeed());
        });

        let snapshot = snapshotter.snapshot().into_vec();
        let (outcome, status_class, failure_category) = attempt.expected_labels();
        let reply_labels = vec![
            Label::new("outcome", outcome),
            Label::new("status_class", status_class),
            Label::new("failure_category", failure_category),
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
                Key::from_parts(REQUEST_DURATION, reply_labels.clone()),
            ) && *unit == Some(Unit::Seconds)
                && matches!(value, DebugValue::Histogram(samples) if samples.len() == 1)
        }));
        let has_timeout_counter = snapshot.iter().any(|(key, _, _, value)| {
            *key == metrics_util::CompositeKey::new(
                MetricKind::Counter,
                Key::from_name(TIMEOUT_COUNT),
            ) && *value == DebugValue::Counter(1)
        });
        assert_eq!(has_timeout_counter, attempt.is_timeout());
    }
}

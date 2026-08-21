//! Pure invariants for the REST review-comment reply boundary.
//!
//! The REST transport owns HTTP I/O, while this module owns the two stable
//! decisions that are useful to test independently: base-URI normalization and
//! reply-status handling.

use http::StatusCode;

/// The application outcome assigned to an HTTP response.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(super) enum ReplyStatus {
    /// A successful response permits resolution to continue.
    Success,
    /// A missing reply target is warned about and permits resolution to continue.
    NotFound,
    /// Any other response aborts resolution.
    Failure,
}

/// Removes every trailing slash from an API base URI.
pub(super) fn normalize_api_base_url(mut base: String) -> String {
    while base.ends_with('/') {
        base.pop();
    }
    base
}

/// Classifies a reply response according to `vk`'s documented semantics.
pub(super) fn classify_reply_status(status: StatusCode) -> ReplyStatus {
    if status == StatusCode::NOT_FOUND {
        ReplyStatus::NotFound
    } else if status.is_success() {
        ReplyStatus::Success
    } else {
        ReplyStatus::Failure
    }
}

/// Returns the bounded HTTP status-class label for a response.
pub(super) fn status_class(status: StatusCode) -> &'static str {
    match status.as_u16() {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    //! Tests for REST reply invariants.

    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn normalizes_every_trailing_slash_count(
            host in "[a-z]{1,20}",
            path_segments in proptest::collection::vec("[a-z]{1,20}", 0..4),
            trailing_slashes in 0_usize..65,
        ) {
            let mut base = format!("https://{host}");
            for segment in path_segments {
                base.push('/');
                base.push_str(&segment);
            }
            let normalized = normalize_api_base_url(format!(
                "{base}{}",
                "/".repeat(trailing_slashes),
            ));

            prop_assert_eq!(normalized.clone(), base);
            prop_assert_eq!(normalize_api_base_url(normalized.clone()), normalized);
        }

        #[test]
        fn classifies_every_valid_http_status(status_code in 100_u16..1000) {
            let status = StatusCode::from_u16(status_code).expect("generated standard status");
            let expected_outcome = match status_code {
                404 => ReplyStatus::NotFound,
                200..=299 => ReplyStatus::Success,
                _ => ReplyStatus::Failure,
            };
            let expected_class = match status_code {
                100..=199 => "1xx",
                200..=299 => "2xx",
                300..=399 => "3xx",
                400..=499 => "4xx",
                500..=599 => "5xx",
                _ => "other",
            };

            prop_assert_eq!(classify_reply_status(status), expected_outcome);
            prop_assert_eq!(status_class(status), expected_class);
        }
    }
}

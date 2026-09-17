//! Tests for pagination helpers.
//!
//! The traversal behaviours (item concatenation, error discarding, cursor
//! advancement) are covered by the `paginate_operation` tests in
//! `crate::api::client::pagination`; this module covers the [`PageInfo`]
//! cursor invariants those traversals rely on.

use crate::{PageInfo, VkError};
use proptest::prelude::*;
use rstest::rstest;

#[rstest]
#[case(false, None, None)]
#[case(false, Some(String::from("extra")), None)]
#[case(true, Some(String::from("abc")), Some("abc"))]
fn next_cursor_ok_cases(
    #[case] has_next_page: bool,
    #[case] end_cursor: Option<String>,
    #[case] expected: Option<&str>,
) {
    let info = PageInfo {
        has_next_page,
        end_cursor,
    };
    let next = info.next_cursor().expect("cursor");
    assert_eq!(next, expected);
}

#[test]
fn next_cursor_errors_without_cursor() {
    let info = PageInfo {
        has_next_page: true,
        end_cursor: None,
    };
    let err = info.next_cursor().expect_err("missing cursor");
    assert!(matches!(err, VkError::BadResponse(_)));
}

proptest! {
    #[test]
    fn page_info_terminates_or_replaces_the_cursor(
        has_next_page in any::<bool>(),
        end_cursor in proptest::option::of("[a-z0-9]{1,16}"),
    ) {
        let page = PageInfo {
            has_next_page,
            end_cursor: end_cursor.clone(),
        };
        let cursor = page.next_cursor();

        if has_next_page {
            if let Some(expected) = end_cursor {
                prop_assert_eq!(cursor.expect("cursor"), Some(expected.as_str()));
            } else {
                prop_assert!(matches!(cursor, Err(VkError::BadResponse(_))));
            }
        } else {
            prop_assert_eq!(cursor.expect("terminal page"), None);
        }
    }
}

//! Tests for pagination helpers.

use super::paginate;
use crate::{PageInfo, VkError};
use rstest::rstest;
use std::cell::RefCell;

#[tokio::test]
async fn paginate_discards_items_on_error() {
    let seen = RefCell::new(Vec::new());

    let result: Result<Vec<i32>, VkError> = paginate(|cursor| {
        let seen = &seen;
        async move {
            if cursor.is_none() {
                seen.borrow_mut().push(1);
                Ok((
                    vec![1],
                    PageInfo {
                        has_next_page: true,
                        end_cursor: Some("next".to_string()),
                    },
                ))
            } else {
                Err(VkError::ApiErrors("boom".into()))
            }
        }
    })
    .await;

    assert!(result.is_err());
    assert_eq!(seen.borrow().as_slice(), &[1]);
}

#[tokio::test]
async fn paginate_missing_cursor_errors() {
    let result: Result<Vec<i32>, VkError> = paginate(|_cursor| async {
        Ok((
            vec![1],
            PageInfo {
                has_next_page: true,
                end_cursor: None,
            },
        ))
    })
    .await;
    match result {
        Err(VkError::BadResponse(msg)) => {
            let s = msg.to_string();
            assert!(
                s.contains("hasNextPage=true") && s.contains("endCursor"),
                "{s}"
            );
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

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

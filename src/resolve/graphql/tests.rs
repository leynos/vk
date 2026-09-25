//! Unit tests for GraphQL review-thread resolution.

use super::*;
use crate::ref_parser::RepoInfo;
use mockall::Sequence;
use rstest::rstest;
use thread_for_comment_query::{
    ResponseData, ThreadForCommentQueryRepository, ThreadForCommentQueryRepositoryPullRequest,
    ThreadForCommentQueryRepositoryPullRequestReviewThreads,
    ThreadForCommentQueryRepositoryPullRequestReviewThreadsNodes,
    ThreadForCommentQueryRepositoryPullRequestReviewThreadsNodesComments,
    ThreadForCommentQueryRepositoryPullRequestReviewThreadsNodesCommentsNodes,
    ThreadForCommentQueryRepositoryPullRequestReviewThreadsNodesCommentsPageInfo,
    ThreadForCommentQueryRepositoryPullRequestReviewThreadsPageInfo,
};

/// Build one thread node containing the given comment database ids.
fn thread(
    id: &str,
    comment_ids: Vec<u64>,
) -> ThreadForCommentQueryRepositoryPullRequestReviewThreadsNodes {
    ThreadForCommentQueryRepositoryPullRequestReviewThreadsNodes {
        id: id.into(),
        comments: ThreadForCommentQueryRepositoryPullRequestReviewThreadsNodesComments {
            nodes: Some(
                comment_ids
                    .into_iter()
                    .map(|database_id| {
                        Some(ThreadForCommentQueryRepositoryPullRequestReviewThreadsNodesCommentsNodes {
                            full_database_id: Some(database_id.to_string()),
                        })
                    })
                    .collect(),
            ),
            page_info:
                ThreadForCommentQueryRepositoryPullRequestReviewThreadsNodesCommentsPageInfo {
                    end_cursor: None,
                    has_next_page: false,
                },
        },
    }
}

/// Build a page of review threads, each holding one comment id, with the
/// given pagination cursor.
fn page(comment_ids: Vec<u64>, end_cursor: Option<&str>, has_next: bool) -> ThreadPage {
    ResponseData {
        repository: Some(ThreadForCommentQueryRepository {
            pull_request: Some(ThreadForCommentQueryRepositoryPullRequest {
                review_threads: ThreadForCommentQueryRepositoryPullRequestReviewThreads {
                    page_info: ThreadForCommentQueryRepositoryPullRequestReviewThreadsPageInfo {
                        end_cursor: end_cursor.map(ToOwned::to_owned),
                        has_next_page: has_next,
                    },
                    nodes: Some(
                        comment_ids
                            .into_iter()
                            .map(|id| Some(thread(&format!("thread-{id}"), vec![id])))
                            .collect(),
                    ),
                },
            }),
        }),
    }
}

#[test]
fn next_thread_cursor_finishes_without_a_next_page() {
    let page_info = ThreadForCommentQueryRepositoryPullRequestReviewThreadsPageInfo {
        end_cursor: Some("unused".to_string()),
        has_next_page: false,
    };

    assert!(matches!(
        next_thread_cursor(&page_info, &mut CursorHistory::new(Some("previous")),),
        Ok(None)
    ));
}

#[rstest]
#[case::missing_end_cursor(
    ThreadForCommentQueryRepositoryPullRequestReviewThreadsPageInfo {
        end_cursor: None,
        has_next_page: true,
    },
    Some("previous".to_string()),
    "missing endCursor with hasNextPage"
)]
#[case::repeated_cursor(
    ThreadForCommentQueryRepositoryPullRequestReviewThreadsPageInfo {
        end_cursor: Some("previous".to_string()),
        has_next_page: true,
    },
    Some("previous".to_string()),
    "non-progressing pagination (repeated endCursor)"
)]
fn next_thread_cursor_rejects_invalid_progress(
    #[case] page_info: ThreadForCommentQueryRepositoryPullRequestReviewThreadsPageInfo,
    #[case] previous: Option<String>,
    #[case] expected: &str,
) {
    assert!(
        matches!(next_thread_cursor(&page_info, &mut CursorHistory::new(previous.as_deref())), Err(VkError::BadResponse(message)) if message.as_ref() == expected)
    );
}

#[rstest]
#[case::truncated(
    true,
    VkError::BadResponse(
        "comment 42 was not found in the fetched first 100 comments of a review thread".into()
    )
)]
#[case::not_found(false, VkError::CommentNotFound { comment_id: 42 })]
fn thread_lookup_error_reports_the_terminal_lookup_result(
    #[case] comments_truncated: bool,
    #[case] expected: VkError,
) {
    assert_eq!(
        format!("{:?}", thread_lookup_error(42, comments_truncated)),
        format!("{expected:?}")
    );
}

#[rstest]
#[case::missing_threads(vec![ResponseData { repository: None }], VkError::BadResponse("missing review threads".into()))]
#[case::missing_cursor(vec![page(vec![], None, true)], VkError::BadResponse("missing endCursor with hasNextPage".into()))]
#[case::repeated_cursor(
    vec![
        page(vec![], Some("a"), true),
        page(vec![], Some("a"), true),
    ],
    VkError::BadResponse("non-progressing pagination (repeated endCursor)".into()),
)]
#[case::cursor_cycle(
    vec![
        page(vec![], Some("a"), true),
        page(vec![], Some("b"), true),
        page(vec![], Some("a"), true),
    ],
    VkError::BadResponse("non-progressing pagination (repeated endCursor)".into()),
)]
#[case::not_found(
    vec![
        page(vec![1], Some("a"), true),
        page(vec![2], None, false),
    ],
    VkError::CommentNotFound { comment_id: 42 },
)]
#[tokio::test]
async fn pagination_errors(#[case] pages: Vec<ThreadPage>, #[case] expected: VkError) {
    let mut mock = MockReviewCommentsFetcher::new();
    let mut seq = Sequence::new();
    for page in pages {
        let p = page.clone();
        mock.expect_fetch_review_comments()
            .times(1)
            .in_sequence(&mut seq)
            .returning(move |_| Ok(p.clone()));
    }
    let repo = RepoInfo {
        owner: "o".into(),
        name: "r".into(),
    };
    let reference = CommentRef {
        repo: &repo,
        pull_number: 1,
        comment_id: 42,
    };
    let err = get_thread_id(&mock, reference)
        .await
        .expect_err("expected error");
    assert_eq!(format!("{err:?}"), format!("{expected:?}"));
}

#[tokio::test]
async fn finds_thread_owning_the_comment() {
    let mut mock = MockReviewCommentsFetcher::new();
    let mut seq = Sequence::new();
    let pages = vec![
        page(vec![1], Some("a"), true),
        // Second page: the thread for comment 42 holds the requested comment.
        page(vec![42], None, false),
    ];
    for page in pages {
        let p = page.clone();
        mock.expect_fetch_review_comments()
            .times(1)
            .in_sequence(&mut seq)
            .returning(move |_| Ok(p.clone()));
    }
    let repo = RepoInfo {
        owner: "o".into(),
        name: "r".into(),
    };
    let reference = CommentRef {
        repo: &repo,
        pull_number: 1,
        comment_id: 42,
    };
    let id = get_thread_id(&mock, reference).await.expect("thread id");
    assert_eq!(id, "thread-42");
}

#[test]
fn finds_comment_in_a_later_thread_on_the_same_page() {
    let threads = vec![
        Some(thread("thread-1", vec![1])),
        Some(thread("thread-42", vec![42])),
    ];

    let (id, truncated) = find_thread_in_page(threads, "42");

    assert_eq!(id.as_deref(), Some("thread-42"));
    assert!(!truncated);
}

#[tokio::test]
async fn reports_truncated_thread_comments_before_comment_not_found() {
    let mut page = page(vec![1], None, false);
    let thread = page
        .repository
        .as_mut()
        .and_then(|repository| repository.pull_request.as_mut())
        .and_then(|pull_request| pull_request.review_threads.nodes.as_mut())
        .and_then(|nodes| nodes.first_mut())
        .and_then(Option::as_mut)
        .expect("thread node");
    thread.comments.page_info.has_next_page = true;
    let mut mock = MockReviewCommentsFetcher::new();
    mock.expect_fetch_review_comments()
        .times(1)
        .returning(move |_| Ok(page.clone()));
    let repo = RepoInfo {
        owner: "o".into(),
        name: "r".into(),
    };

    let error = get_thread_id(
        &mock,
        CommentRef {
            repo: &repo,
            pull_number: 1,
            comment_id: 42,
        },
    )
    .await
    .expect_err("truncated comments must be reported");

    assert!(
        matches!(error, VkError::BadResponse(message) if message.contains("first 100 comments"))
    );
}

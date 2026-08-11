//! GraphQL helpers for resolving review comment threads.
//!
//! The thread lookup pages through `repository.pullRequest.reviewThreads`,
//! scanning each thread's first 100 comments for the requested comment's
//! database id (`fullDatabaseId`; the plain `databaseId` field is deprecated
//! in the schema). An earlier revision paged a flat
//! `PullRequest.reviewComments` connection, but that field does not exist in
//! GitHub's published GraphQL schema — it only ever worked against mocked
//! responses — so the query was redesigned onto the real schema when the
//! operations moved to `graphql_client` codegen.
//!
//! Accepted limitation: a comment beyond the first 100 comments of a single
//! thread will not be found (the per-thread `comments(first: 100)` cap). This
//! is the same class of cap as the old flat query's `first: 100` page size;
//! review threads of that depth are not a practical concern for `vk resolve`.

use super::CommentRef;
// `graphql_client` resolves the `BigInt` scalar (`fullDatabaseId`) to a type
// of the same name in scope of the derive; the shared alias supplies it.
use crate::api::scalars::BigInt;
use crate::{VkError, api::GraphQLClient};
use graphql_client::GraphQLQuery;

/// Typed `ThreadForCommentQuery` operation: pages review threads (with their
/// first 100 comments) so [`get_thread_id`] can locate the thread owning a
/// comment database id.
///
/// Uses the generated `ResponseData` directly: the fetcher trait below is the
/// only consumer, its unit tests construct the generated types via [`tests`]
/// helpers, and the integration stubs supply every selected field.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/schema.docs.graphql",
    query_path = "graphql/resolve.graphql",
    response_derives = "Debug, Clone, PartialEq"
)]
pub(crate) struct ThreadForCommentQuery;

/// Typed `ResolveReviewThreadMutation`: marks a review thread resolved.
///
/// Uses the generated `ResponseData` directly ([`GraphQLClient::run_operation`])
/// since nothing beyond a successful deserialization is consumed.
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/schema.docs.graphql",
    query_path = "graphql/resolve.graphql",
    response_derives = "Debug, Clone, PartialEq"
)]
pub(crate) struct ResolveReviewThreadMutation;

/// One page of thread-for-comment data as returned by the API.
pub(crate) type ThreadPage = thread_for_comment_query::ResponseData;

#[cfg(test)]
use mockall::automock;

#[derive(Debug)]
pub(crate) struct ReviewCommentsQuery<'a> {
    pub owner: &'a str,
    pub name: &'a str,
    pub number: u64,
    pub after: Option<String>,
}

#[cfg_attr(test, automock)]
#[allow(clippy::ref_option, reason = "automock generates &Option")]
pub(crate) trait ReviewCommentsFetcher {
    #[allow(
        clippy::elidable_lifetime_names,
        reason = "automock requires explicit lifetime for query struct"
    )]
    async fn fetch_review_comments<'a>(
        &self,
        query: ReviewCommentsQuery<'a>,
    ) -> Result<ThreadPage, VkError>;
}

impl ReviewCommentsFetcher for GraphQLClient {
    #[allow(
        clippy::elidable_lifetime_names,
        reason = "automock requires explicit lifetime for query struct"
    )]
    async fn fetch_review_comments<'a>(
        &self,
        query: ReviewCommentsQuery<'a>,
    ) -> Result<ThreadPage, VkError> {
        // GraphQL `Int` maps to `i64`; PR numbers fit comfortably, but reject
        // an out-of-range value rather than silently truncating.
        let number = i64::try_from(query.number).map_err(|_| VkError::InvalidNumber)?;
        let variables = thread_for_comment_query::Variables {
            owner: query.owner.to_string(),
            name: query.name.to_string(),
            number,
            after: query.after,
        };
        self.run_operation::<ThreadForCommentQuery>(variables).await
    }
}

/// Scan one page of review threads for the comment with `target` database id,
/// returning the owning thread's id when found.
fn find_thread_in_page(
    threads: Vec<
        Option<
            thread_for_comment_query::ThreadForCommentQueryRepositoryPullRequestReviewThreadsNodes,
        >,
    >,
    target: &str,
) -> Option<String> {
    threads.into_iter().flatten().find_map(|thread| {
        let hit = thread
            .comments
            .nodes
            .iter()
            .flatten()
            .flatten()
            .any(|comment| comment.full_database_id.as_deref() == Some(target));
        hit.then_some(thread.id)
    })
}

pub(crate) async fn get_thread_id(
    gql: &impl ReviewCommentsFetcher,
    reference: CommentRef<'_>,
) -> Result<String, VkError> {
    // Comment ids come from `#discussion_r<ID>` permalinks; `fullDatabaseId`
    // is a `BigInt` scalar carried as a decimal string, so the id is matched
    // by its canonical string form.
    let target = reference.comment_id.to_string();
    let mut cursor: Option<String> = None;
    loop {
        let data = gql
            .fetch_review_comments(ReviewCommentsQuery {
                owner: &reference.repo.owner,
                name: &reference.repo.name,
                number: reference.pull_number,
                after: cursor.clone(),
            })
            .await?;
        let threads = data
            .repository
            .and_then(|r| r.pull_request)
            .map(|p| p.review_threads)
            .ok_or_else(|| VkError::BadResponse("missing review threads".into()))?;
        if let Some(nodes) = threads.nodes
            && let Some(id) = find_thread_in_page(nodes, &target)
        {
            return Ok(id);
        }
        if !threads.page_info.has_next_page {
            break;
        }
        let next = threads.page_info.end_cursor;
        if next.is_none() {
            return Err(VkError::BadResponse(
                "missing endCursor with hasNextPage".into(),
            ));
        }
        if next == cursor {
            return Err(VkError::BadResponse(
                "non-progressing pagination (repeated endCursor)".into(),
            ));
        }
        cursor = next;
    }
    Err(VkError::CommentNotFound {
        comment_id: reference.comment_id,
    })
}

pub(crate) async fn resolve_thread(gql: &GraphQLClient, thread_id: &str) -> Result<(), VkError> {
    // The mutation's response is intentionally unconsumed beyond a successful
    // deserialization, so the generated `ResponseData` is discarded.
    let _ = gql
        .run_operation::<ResolveReviewThreadMutation>(resolve_review_thread_mutation::Variables {
            id: thread_id.to_string(),
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
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
                        page_info:
                            ThreadForCommentQueryRepositoryPullRequestReviewThreadsPageInfo {
                                end_cursor: end_cursor.map(ToOwned::to_owned),
                                has_next_page: has_next,
                            },
                        nodes: Some(
                            comment_ids
                                .into_iter()
                                .map(|id| Some(thread("t", vec![id])))
                                .collect(),
                        ),
                    },
                }),
            }),
        }
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
            // Second page: thread "t" holds the requested comment.
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
        assert_eq!(id, "t");
    }
}

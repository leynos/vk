//! GraphQL helpers for resolving review comment threads.

use super::CommentRef;
use crate::{VkError, api::GraphQLClient};
use serde::Deserialize;
use serde_json::json;

/// GraphQL mutation used to mark a review thread as resolved.
const RESOLVE_THREAD_MUTATION: &str = r"
    mutation($id: ID!) {
      resolveReviewThread(input: {threadId: $id}) { clientMutationId }
    }
";

/// GraphQL query used to fetch one page of review comments.
const REVIEW_COMMENTS_PAGE: &str = r"
    query($owner: String!, $name: String!, $number: Int!, $after: String) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          reviewComments(first: 100, after: $after) {
            pageInfo { endCursor hasNextPage }
            nodes { databaseId pullRequestReviewThread { id } }
          }
        }
      }
    }
";

#[cfg(test)]
use mockall::automock;

#[derive(Debug)]
/// Variables for a paginated review-comment query.
pub(crate) struct ReviewCommentsQuery<'a> {
    /// Repository owner login.
    pub owner: &'a str,
    /// Repository name.
    pub name: &'a str,
    /// Pull-request number.
    pub number: u64,
    /// Cursor returned by the preceding page, if any.
    pub after: Option<String>,
}

#[cfg_attr(test, automock)]
#[allow(clippy::ref_option, reason = "automock generates &Option")]
/// Fetches pages of review comments for a pull request.
pub(crate) trait ReviewCommentsFetcher {
    #[allow(
        clippy::elidable_lifetime_names,
        reason = "automock requires explicit lifetime for query struct"
    )]
    /// Fetch one page of review comments.
    async fn fetch_review_comments<'a>(
        &self,
        query: ReviewCommentsQuery<'a>,
    ) -> Result<ReviewCommentsPage, VkError>;
}

impl ReviewCommentsFetcher for GraphQLClient {
    #[allow(
        clippy::elidable_lifetime_names,
        reason = "automock requires explicit lifetime for query struct"
    )]
    async fn fetch_review_comments<'a>(
        &self,
        query: ReviewCommentsQuery<'a>,
    ) -> Result<ReviewCommentsPage, VkError> {
        self.run_query(
            REVIEW_COMMENTS_PAGE,
            json!({
                "owner": query.owner,
                "name": query.name,
                "number": query.number,
                "after": query.after,
            }),
        )
        .await
    }
}

#[derive(Clone, Deserialize)]
/// GraphQL response containing one page of review comments.
pub(crate) struct ReviewCommentsPage {
    /// Repository data, when the repository exists.
    repository: Option<Repository>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Repository portion of a review-comment response.
pub(crate) struct Repository {
    /// Pull request data, when the pull request exists.
    pull_request: Option<PullRequest>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Pull-request portion of a review-comment response.
pub(crate) struct PullRequest {
    /// Review comments connection, when available.
    review_comments: Option<ReviewComments>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Review-comment connection and pagination information.
pub(crate) struct ReviewComments {
    /// Pagination metadata.
    page_info: PageInfo,
    /// Comments in this page.
    nodes: Vec<CommentNode>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Cursor metadata for a review-comment page.
pub(crate) struct PageInfo {
    /// Cursor for the next page, when present.
    end_cursor: Option<String>,
    /// Whether another page is available.
    has_next_page: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Review-comment node carrying its database and thread identifiers.
pub(crate) struct CommentNode {
    /// GitHub database identifier for the comment.
    database_id: u64,
    /// Thread associated with the comment.
    pull_request_review_thread: ReviewThread,
}

#[derive(Clone, Deserialize)]
/// GraphQL representation of a review thread.
pub(crate) struct ReviewThread {
    /// Global GraphQL identifier for the thread.
    id: String,
}

#[derive(Clone, Deserialize)]
/// Response returned by the resolve-thread mutation.
pub(crate) struct ResolveThreadResponse {
    #[serde(rename = "resolveReviewThread")]
    /// Mutation payload, when GitHub returns one.
    _resolve_review_thread: Option<ResolveThreadInner>,
}

#[derive(Clone, Deserialize)]
/// Payload returned by the resolve-thread mutation.
pub(crate) struct ResolveThreadInner {
    #[serde(rename = "clientMutationId")]
    /// Client mutation identifier returned by GitHub.
    _client_mutation_id: Option<String>,
}

/// Find the GraphQL thread identifier for a review comment.
pub(crate) async fn get_thread_id(
    gql: &impl ReviewCommentsFetcher,
    reference: CommentRef<'_>,
) -> Result<String, VkError> {
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
        let comments = data
            .repository
            .and_then(|r| r.pull_request)
            .and_then(|p| p.review_comments)
            .ok_or_else(|| VkError::BadResponse("missing review comments".into()))?;
        if let Some(node) = comments
            .nodes
            .iter()
            .find(|n| n.database_id == reference.comment_id)
        {
            return Ok(node.pull_request_review_thread.id.clone());
        }
        if !comments.page_info.has_next_page {
            break;
        }
        let next = comments.page_info.end_cursor.clone();
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

/// Mark a GraphQL review thread as resolved.
pub(crate) async fn resolve_thread(gql: &GraphQLClient, thread_id: &str) -> Result<(), VkError> {
    let _: ResolveThreadResponse = gql
        .run_query(RESOLVE_THREAD_MUTATION, json!({ "id": thread_id }))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ref_parser::RepoInfo;
    use mockall::Sequence;
    use rstest::rstest;

    fn page(nodes: Vec<u64>, end_cursor: Option<&str>, has_next: bool) -> ReviewCommentsPage {
        ReviewCommentsPage {
            repository: Some(Repository {
                pull_request: Some(PullRequest {
                    review_comments: Some(ReviewComments {
                        page_info: PageInfo {
                            end_cursor: end_cursor.map(ToOwned::to_owned),
                            has_next_page: has_next,
                        },
                        nodes: nodes
                            .into_iter()
                            .map(|id| CommentNode {
                                database_id: id,
                                pull_request_review_thread: ReviewThread { id: "t".into() },
                            })
                            .collect(),
                    }),
                }),
            }),
        }
    }

    #[rstest]
    #[case::missing_comments(vec![ReviewCommentsPage { repository: None }], VkError::BadResponse("missing review comments".into()))]
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
    async fn pagination_errors(#[case] pages: Vec<ReviewCommentsPage>, #[case] expected: VkError) {
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
}

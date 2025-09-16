//! GraphQL helpers for resolving review comment threads.

use super::CommentRef;
use crate::{VkError, api::GraphQLClient};
use serde::Deserialize;
use serde_json::json;

const RESOLVE_THREAD_MUTATION: &str = r"
    mutation($id: ID!) {
      resolveReviewThread(input: {threadId: $id}) { clientMutationId }
    }
";

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
pub(crate) struct ReviewCommentsPage {
    repository: Option<Repository>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Repository {
    pull_request: Option<PullRequest>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PullRequest {
    review_comments: Option<ReviewComments>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewComments {
    page_info: PageInfo,
    nodes: Vec<CommentNode>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageInfo {
    end_cursor: Option<String>,
    has_next_page: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommentNode {
    database_id: u64,
    pull_request_review_thread: ReviewThread,
}

#[derive(Clone, Deserialize)]
pub(crate) struct ReviewThread {
    id: String,
}

#[derive(Clone, Deserialize)]
pub(crate) struct ResolveThreadResponse {
    #[serde(rename = "resolveReviewThread")]
    _resolve_review_thread: Option<ResolveThreadInner>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct ResolveThreadInner {
    #[serde(rename = "clientMutationId")]
    _client_mutation_id: Option<String>,
}

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

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

#[derive(Deserialize)]
struct ReviewCommentsPage {
    repository: Option<Repository>,
}

#[derive(Deserialize)]
struct Repository {
    #[serde(rename = "pullRequest")]
    pull_request: Option<PullRequest>,
}

#[derive(Deserialize)]
struct PullRequest {
    #[serde(rename = "reviewComments")]
    review_comments: ReviewComments,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewComments {
    page_info: PageInfo,
    nodes: Vec<CommentNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    end_cursor: Option<String>,
    has_next_page: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentNode {
    database_id: u64,
    pull_request_review_thread: ReviewThread,
}

#[derive(Deserialize)]
struct ReviewThread {
    id: String,
}

#[derive(Deserialize)]
struct ResolveThreadResponse {
    #[serde(rename = "resolveReviewThread")]
    _resolve_review_thread: Option<ResolveThreadInner>,
}

#[derive(Deserialize)]
struct ResolveThreadInner {
    #[serde(rename = "clientMutationId")]
    _client_mutation_id: Option<String>,
}

pub(crate) async fn get_thread_id(
    gql: &GraphQLClient,
    reference: CommentRef<'_>,
) -> Result<String, VkError> {
    let mut cursor = None;
    loop {
        let data: ReviewCommentsPage = gql
            .run_query(
                REVIEW_COMMENTS_PAGE,
                json!({
                    "owner": reference.repo.owner,
                    "name": reference.repo.name,
                    "number": reference.pull_number,
                    "after": cursor,
                }),
            )
            .await?;
        let comments = data
            .repository
            .and_then(|r| r.pull_request)
            .map(|p| p.review_comments)
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
        cursor = comments.page_info.end_cursor;
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

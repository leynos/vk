//! Data structures for GraphQL responses.

use serde::Deserialize;

#[derive(Deserialize)]
pub struct ThreadData {
    pub repository: Repository,
}

#[derive(Deserialize)]
pub struct Repository {
    #[serde(rename = "pullRequest")]
    pub pull_request: PullRequest,
}

#[derive(Deserialize)]
pub struct PullRequest {
    #[serde(rename = "reviewThreads")]
    pub review_threads: ReviewThreadConnection,
}

#[derive(Deserialize)]
pub struct IssueData {
    pub repository: IssueRepository,
}

#[derive(Deserialize)]
pub struct IssueRepository {
    pub issue: Issue,
}

#[derive(Deserialize)]
pub struct Issue {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct ReviewThreadConnection {
    pub nodes: Vec<ReviewThread>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Debug, Deserialize, Default)]
pub struct ReviewThread {
    pub id: String,
    #[serde(rename = "isResolved")]
    #[allow(
        dead_code,
        reason = "GraphQL query requires this field but it is unused"
    )]
    pub is_resolved: bool,
    pub comments: CommentConnection,
}

#[derive(Debug, Deserialize, Default)]
pub struct CommentConnection {
    pub nodes: Vec<ReviewComment>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Debug, Deserialize, Default)]
pub struct ReviewComment {
    pub body: String,
    #[serde(rename = "diffHunk")]
    pub diff_hunk: String,
    #[serde(rename = "originalPosition")]
    pub original_position: Option<i32>,
    pub position: Option<i32>,
    #[allow(dead_code, reason = "stored for completeness; not displayed yet")]
    pub path: String,
    pub url: String,
    pub author: Option<User>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PageInfo {
    #[serde(rename = "hasNextPage")]
    pub has_next_page: bool,
    #[serde(rename = "endCursor")]
    pub end_cursor: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct User {
    pub login: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct CommentNodeWrapper {
    pub node: Option<CommentNode>,
}

#[derive(Debug, Deserialize, Default)]
pub struct CommentNode {
    pub comments: CommentConnection,
}

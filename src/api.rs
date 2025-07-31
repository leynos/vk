//! GitHub GraphQL client and query helpers.
#![allow(
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::collapsible_if,
    reason = "internal helpers"
)]

use log::warn;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, USER_AGENT};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use std::env;

use crate::ref_utils::{
    CommentConnection, CommentNodeWrapper, Issue, IssueData, PageInfo, RepoInfo, ReviewComment,
    ReviewThread, ThreadData, VkError,
};
const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";
const BODY_SNIPPET_LEN: usize = 500;
const VALUE_SNIPPET_LEN: usize = 200;

fn snippet(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out = text.chars().take(max).collect::<String>();
        out.push_str("...");
        out
    }
}

#[derive(Debug, Deserialize)]
struct GraphQlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: String,
}

fn handle_graphql_errors(errors: Vec<GraphQlError>) -> VkError {
    let msg = errors
        .into_iter()
        .map(|e| e.message)
        .collect::<Vec<_>>()
        .join(", ");
    VkError::ApiErrors(msg)
}

/// GitHub GraphQL client with optional HTTP transcript output.
pub struct GraphQLClient {
    client: reqwest::Client,
    headers: HeaderMap,
    endpoint: String,
    transcript: Option<std::sync::Mutex<std::io::BufWriter<std::fs::File>>>,
}

impl GraphQLClient {
    pub fn new(
        token: &str,
        transcript: Option<std::path::PathBuf>,
    ) -> Result<Self, std::io::Error> {
        let endpoint =
            env::var("GITHUB_GRAPHQL_URL").unwrap_or_else(|_| GITHUB_GRAPHQL_URL.to_string());
        Self::with_endpoint(token, &endpoint, transcript)
    }

    pub fn with_endpoint(
        token: &str,
        endpoint: &str,
        transcript: Option<std::path::PathBuf>,
    ) -> Result<Self, std::io::Error> {
        let transcript = match transcript {
            Some(p) => match std::fs::File::create(p) {
                Ok(file) => Some(std::sync::Mutex::new(std::io::BufWriter::new(file))),
                Err(e) => return Err(e),
            },
            None => None,
        };
        Ok(Self {
            client: reqwest::Client::new(),
            headers: build_headers(token),
            endpoint: endpoint.to_string(),
            transcript,
        })
    }

    pub async fn run_query<V, T>(&self, query: &str, variables: V) -> Result<T, VkError>
    where
        V: serde::Serialize,
        T: DeserializeOwned,
    {
        let payload = json!({ "query": query, "variables": &variables });
        let ctx = serde_json::to_string(&payload).unwrap_or_default();
        let response = self
            .client
            .post(&self.endpoint)
            .headers(self.headers.clone())
            .json(&payload)
            .send()
            .await
            .map_err(|e| VkError::RequestContext {
                context: ctx.clone(),
                source: e,
            })?;
        let body = response.text().await.map_err(|e| VkError::RequestContext {
            context: ctx.clone(),
            source: e,
        })?;
        if let Some(t) = &self.transcript {
            use std::io::Write as _;
            if let Ok(mut f) = t.lock() {
                if let Err(e) = writeln!(
                    f,
                    "{}",
                    serde_json::to_string(&json!({ "request": payload, "response": body }))
                        .unwrap_or_default()
                ) {
                    warn!("failed to write transcript: {e}");
                }
            }
        }
        let resp: GraphQlResponse<serde_json::Value> =
            serde_json::from_str(&body).map_err(|e| {
                let snippet = snippet(&body, BODY_SNIPPET_LEN);
                VkError::BadResponseSerde(format!("{e} | response body snippet:{snippet}"))
            })?;

        let resp_debug = format!("{resp:?}");
        if let Some(errs) = resp.errors {
            return Err(handle_graphql_errors(errs));
        }

        let value = resp.data.ok_or_else(|| {
            VkError::BadResponse(format!("Missing data in response: {resp_debug}"))
        })?;
        match serde_path_to_error::deserialize::<_, T>(value.clone()) {
            Ok(v) => Ok(v),
            Err(e) => {
                let snippet = snippet(
                    &serde_json::to_string_pretty(&value).unwrap_or_default(),
                    VALUE_SNIPPET_LEN,
                );
                let path = e.path().to_string();
                let inner = e.into_inner();
                Err(VkError::BadResponseSerde(format!(
                    "{inner} at {path} | snippet: {snippet}"
                )))
            }
        }
    }
}

pub(crate) async fn paginate<T, F, Fut>(mut fetch: F) -> Result<Vec<T>, VkError>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<T>, PageInfo), VkError>>,
{
    let mut items = Vec::new();
    let mut cursor = None;
    loop {
        let (mut page, info) = fetch(cursor.clone()).await?;
        items.append(&mut page);
        if !info.has_next_page {
            break;
        }
        cursor = info.end_cursor;
    }
    Ok(items)
}

async fn fetch_comment_page(
    client: &GraphQLClient,
    id: &str,
    cursor: Option<String>,
) -> Result<(Vec<ReviewComment>, PageInfo), VkError> {
    let wrapper: CommentNodeWrapper = client
        .run_query(COMMENT_QUERY, json!({ "id": id, "cursor": cursor.clone() }))
        .await?;
    let conn = wrapper
        .node
        .ok_or_else(|| {
            VkError::BadResponse(format!(
                "Missing comment node in response (id: {}, cursor: {})",
                id,
                cursor.as_deref().unwrap_or("None")
            ))
        })?
        .comments;
    Ok((conn.nodes, conn.page_info))
}

pub async fn fetch_issue(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Issue, VkError> {
    let data: IssueData = client
        .run_query(
            ISSUE_QUERY,
            json!({ "owner": repo.owner.as_str(), "name": repo.name.as_str(), "number": number }),
        )
        .await?;
    Ok(data.repository.issue)
}

async fn fetch_thread_page(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
    cursor: Option<String>,
) -> Result<(Vec<ReviewThread>, PageInfo), VkError> {
    let data: ThreadData = client
        .run_query(
            THREADS_QUERY,
            json!({ "owner": repo.owner.as_str(), "name": repo.name.as_str(), "number": number, "cursor": cursor }),
        )
        .await?;
    let conn = data.repository.pull_request.review_threads;
    Ok((conn.nodes, conn.page_info))
}

pub async fn fetch_review_threads(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Vec<ReviewThread>, VkError> {
    let mut threads = paginate(|cursor| fetch_thread_page(client, repo, number, cursor)).await?;
    threads.retain(|t| !t.is_resolved);

    for thread in &mut threads {
        let initial = std::mem::replace(
            &mut thread.comments,
            CommentConnection {
                nodes: Vec::new(),
                page_info: PageInfo {
                    has_next_page: false,
                    end_cursor: None,
                },
            },
        );
        let mut comments = initial.nodes;
        if initial.page_info.has_next_page {
            let more = paginate(|c| fetch_comment_page(client, &thread.id, c)).await?;
            comments.extend(more);
        }
        thread.comments = CommentConnection {
            nodes: comments,
            page_info: PageInfo {
                has_next_page: false,
                end_cursor: None,
            },
        };
    }
    Ok(threads)
}

fn build_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, "vk".parse().expect("static string"));
    headers.insert(
        ACCEPT,
        "application/vnd.github+json"
            .parse()
            .expect("static string"),
    );
    if !token.is_empty() {
        headers.insert(
            AUTHORIZATION,
            format!("Bearer {token}").parse().expect("valid header"),
        );
    }
    headers
}

#[expect(
    clippy::result_large_err,
    reason = "VkError has many variants but they are small"
)]
pub fn build_graphql_client(
    token: &str,
    transcript: Option<&std::path::PathBuf>,
) -> Result<GraphQLClient, VkError> {
    match GraphQLClient::new(token, transcript.cloned()) {
        Ok(c) => Ok(c),
        Err(e) => {
            warn!("failed to create transcript: {e}");
            GraphQLClient::new(token, None).map_err(Into::into)
        }
    }
}

const THREADS_QUERY: &str = r"query($owner: String!, $name: String!, $number: Int!, $cursor: String) { repository(owner: $owner, name: $name) { pullRequest(number: $number) { reviewThreads(first: 100, after: $cursor) { nodes { id isResolved comments(first: 100) { nodes { body diffHunk originalPosition position path url author { login } } pageInfo { hasNextPage endCursor } } } pageInfo { hasNextPage endCursor } } } } }";

const COMMENT_QUERY: &str = r"query($id: ID!, $cursor: String) { node(id: $id) { ... on PullRequestReviewThread { comments(first: 100, after: $cursor) { nodes { body diffHunk originalPosition position path url author { login } } pageInfo { hasNextPage endCursor } } } } }";

const ISSUE_QUERY: &str = r"query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { issue(number: $number) { title body } } }";

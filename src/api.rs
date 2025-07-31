//! GraphQL client and query helpers.

use log::warn;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, USER_AGENT};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use thiserror::Error;

use crate::models::ThreadData;
use crate::models::{
    CommentConnection, CommentNodeWrapper, Issue, IssueData, PageInfo, ReviewComment, ReviewThread,
};
use crate::references::RepoInfo;

/// Default endpoint for GitHub's GraphQL API.
pub const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";

#[derive(Error, Debug)]
pub enum VkError {
    #[error("unable to determine repository")]
    RepoNotFound,
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("request failed when running {context}: {source}")]
    RequestContext {
        context: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("invalid reference")]
    InvalidRef,
    #[error("expected URL path segment in {expected:?}, found '{found}'")]
    WrongResourceType {
        expected: &'static [&'static str],
        found: String,
    },
    #[error("bad response: {0}")]
    BadResponse(String),
    #[error("malformed response: {0}")]
    BadResponseSerde(String),
    #[error("API errors: {0}")]
    ApiErrors(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("configuration error: {0}")]
    Config(#[from] ortho_config::OrthoError),
}

fn handle_graphql_errors(errors: Vec<GraphQlError>) -> VkError {
    let msg = errors
        .into_iter()
        .map(|e| e.message)
        .collect::<Vec<_>>()
        .join(", ");
    VkError::ApiErrors(msg)
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

/// A lightweight wrapper around `reqwest::Client` configured for GitHub's GraphQL API.
pub struct GraphQLClient {
    client: reqwest::Client,
    headers: HeaderMap,
    endpoint: String,
    transcript: Option<std::sync::Mutex<std::io::BufWriter<std::fs::File>>>,
}

impl GraphQLClient {
    /// Create a client using the default GitHub endpoint.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` if the transcript file cannot be created.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// let client = vk::api::GraphQLClient::new("token", None).unwrap();
    /// ```
    pub fn new(
        token: &str,
        transcript: Option<std::path::PathBuf>,
    ) -> Result<Self, std::io::Error> {
        Self::with_endpoint(token, GITHUB_GRAPHQL_URL, transcript)
    }

    /// Create a client targeting a custom endpoint.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` if the transcript file or HTTP client cannot
    /// be created. The `endpoint` is useful for tests that mock the GitHub API.
    pub fn with_endpoint(
        token: &str,
        endpoint: &str,
        transcript: Option<std::path::PathBuf>,
    ) -> Result<Self, std::io::Error> {
        let transcript = transcript
            .map(|p| {
                std::fs::File::create(p)
                    .map(|file| std::sync::Mutex::new(std::io::BufWriter::new(file)))
            })
            .transpose()?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(std::io::Error::other)?;
        Ok(Self {
            client,
            headers: build_headers(token),
            endpoint: endpoint.to_string(),
            transcript,
        })
    }

    /// Execute a GraphQL query and deserialize the response.
    ///
    /// # Errors
    ///
    /// Returns `VkError` if the request fails or the response cannot be
    /// deserialized.
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
            match t.lock() {
                Ok(mut f) => {
                    if let Err(e) = writeln!(
                        f,
                        "{}",
                        serde_json::to_string(&json!({ "request": payload, "response": body }))
                            .unwrap_or_default(),
                    ) {
                        warn!("failed to write transcript: {e}");
                    }
                }
                Err(_) => warn!("failed to lock transcript"),
            }
        }
        let resp: GraphQlResponse<serde_json::Value> =
            serde_json::from_str(&body).map_err(|e| {
                let snippet = if body.len() > 500 {
                    let preview: String = body.chars().take(500).collect();
                    format!("{preview}...")
                } else {
                    body.clone()
                };
                VkError::BadResponseSerde(format!("{e} | response body snippet:{snippet}"))
            })?;
        let resp_debug = format!("{resp:?}");
        if let Some(errs) = resp.errors {
            return Err(handle_graphql_errors(errs));
        }
        let value = resp.data.ok_or_else(|| {
            VkError::BadResponse(format!("Missing data in response: {resp_debug}"))
        })?;
        serde_json::from_value(value).map_err(|e| VkError::BadResponseSerde(e.to_string()))
    }
}

/// Construct HTTP headers for GitHub API requests.
///
/// # Panics
///
/// Panics if static strings fail to parse as header values.
pub fn build_headers(token: &str) -> HeaderMap {
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
/// Build a `GraphQLClient` and fall back if transcript creation fails.
///
/// # Errors
///
/// Returns `VkError` if the client or transcript file cannot be created.
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

pub const THREADS_QUERY: &str = r"
    query($owner: String!, $name: String!, $number: Int!, $cursor: String) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          reviewThreads(first: 100, after: $cursor) {
            nodes {
              id
              isResolved
              comments(first: 100) {
                nodes {
                  body
                  diffHunk
                  originalPosition
                  position
                  path
                  url
                  author { login }
                }
                pageInfo { hasNextPage endCursor }
              }
            }
            pageInfo { hasNextPage endCursor }
          }
        }
      }
    }
";

const COMMENT_QUERY: &str = r"
    query($id: ID!, $cursor: String) {
      node(id: $id) {
        ... on PullRequestReviewThread {
          comments(first: 100, after: $cursor) {
            nodes {
              body
              diffHunk
              originalPosition
              position
              path
              url
              author { login }
            }
            pageInfo { hasNextPage endCursor }
          }
        }
      }
    }
";

const ISSUE_QUERY: &str = r"
    query($owner: String!, $name: String!, $number: Int!) {
      repository(owner: $owner, name: $name) {
        issue(number: $number) {
          title
          body
        }
      }
    }
";

/// Retrieve all pages from a paginated GraphQL endpoint.
///
/// The `fetch` closure should request a single page and return the items along
/// with pagination info.
///
/// # Errors
///
/// Propagates any `VkError` returned by the `fetch` closure.
pub async fn paginate<T, F, Fut>(mut fetch: F) -> Result<Vec<T>, VkError>
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

/// Fetch a GitHub issue by number.
///
/// # Errors
///
/// Returns `VkError` if the request or deserialisation fails.
pub async fn fetch_issue(
    client: &GraphQLClient,
    repo: &RepoInfo,
    number: u64,
) -> Result<Issue, VkError> {
    let data: IssueData = client
        .run_query(
            ISSUE_QUERY,
            json!({
                "owner": repo.owner.as_str(),
                "name": repo.name.as_str(),
                "number": number
            }),
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
            json!({
                "owner": repo.owner.as_str(),
                "name": repo.name.as_str(),
                "number": number,
                "cursor": cursor,
            }),
        )
        .await?;
    let conn = data.repository.pull_request.review_threads;
    Ok((conn.nodes, conn.page_info))
}

/// Retrieve all unresolved review threads for a pull request.
///
/// # Errors
///
/// Returns `VkError` if any API call fails.
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

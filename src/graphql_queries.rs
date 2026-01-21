//! GraphQL query strings used to fetch issues and review threads.

pub const THREADS_QUERY: &str = r"
    query($owner: String!, $name: String!, $number: Int!, $cursor: String) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          reviewThreads(first: 100, after: $cursor) {
            nodes {
              id
              isResolved
                isOutdated
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

pub const COMMENT_QUERY: &str = r"
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

pub const ISSUE_QUERY: &str = r"
    query($owner: String!, $name: String!, $number: Int!) {
      repository(owner: $owner, name: $name) {
        issue(number: $number) {
          title
          body
        }
      }
    }
";

/// Query to find PRs by head branch name.
///
/// Fetches up to 10 PRs with the given head ref name to support disambiguation
/// when multiple forks have PRs with the same branch name. The response includes
/// the head repository owner for client-side filtering.
pub const PR_FOR_BRANCH_QUERY: &str = r"
    query($owner: String!, $name: String!, $headRef: String!) {
      repository(owner: $owner, name: $name) {
        pullRequests(headRefName: $headRef, first: 10, states: [OPEN, MERGED]) {
          nodes {
            number
            headRepository {
              owner { login }
            }
          }
        }
      }
    }
";

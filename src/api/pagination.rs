//! Pagination helpers for cursor-based GraphQL connections.

use crate::VkError;
use crate::boxed::BoxedStr;
use std::collections::HashSet;

/// Maximum number of pages fetched by one pagination traversal.
pub(crate) const MAX_PAGES: usize = 1000;

/// Cursor history shared by the crate's GraphQL pagination traversals.
///
/// It owns the cycle-detection invariant for paginators that advance through
/// opaque cursors. Callers seed it with an optional initial cursor and record
/// each cursor returned by the server before making the next request.
pub(crate) struct CursorHistory {
    /// Cursors that have already been used or returned by this traversal.
    seen: HashSet<String>,
}

impl CursorHistory {
    /// Create a history seeded with the cursor used for the first request.
    pub(crate) fn new(initial_cursor: Option<&str>) -> Self {
        Self {
            seen: initial_cursor.into_iter().map(ToOwned::to_owned).collect(),
        }
    }

    /// Record a returned cursor, rejecting a cursor that would form a cycle.
    pub(crate) fn record_next(&mut self, cursor: &str) -> Result<(), VkError> {
        if self.seen.insert(cursor.to_string()) {
            Ok(())
        } else {
            Err(VkError::BadResponse(
                "non-progressing pagination (repeated endCursor)".boxed(),
            ))
        }
    }
}

/// Return whether a one-based page count has exceeded the traversal bound.
pub(crate) const fn page_limit_exceeded(pages_seen: usize) -> bool {
    pages_seen > MAX_PAGES
}

/// Build the shared error returned when a pagination traversal exceeds its cap.
pub(crate) fn page_limit_error() -> VkError {
    VkError::BadResponse(format!("pagination exceeded max pages {MAX_PAGES}").boxed())
}

/// Set the pagination cursor on a generated `Variables` struct.
///
/// `graphql_client` renders each operation's variables as a typed struct, so
/// cursor injection cannot mutate an untyped JSON map. Paginated operations
/// implement this trait so [`super::GraphQLClient::paginate_operation_as`] can
/// advance the cursor between pages without knowing the concrete variables
/// type.
///
/// Passing `None` clears the cursor, requesting the first page.
pub(crate) trait CursorVariables {
    /// Replace the `after`/`cursor` variable with `cursor`.
    fn set_cursor(&mut self, cursor: Option<String>);
}

#[cfg(test)]
mod tests;

//! Pagination helpers for cursor-based GraphQL connections.

/// Set the pagination cursor on a generated `Variables` struct.
///
/// `graphql_client` renders each operation's variables as a typed struct, so
/// cursor injection cannot mutate an untyped JSON map. Paginated operations
/// implement this trait so [`super::GraphQLClient::paginate_operation`] (and
/// its `paginate_operation_as` counterpart) can advance the cursor between
/// pages without knowing the concrete variables type.
///
/// Passing `None` clears the cursor, requesting the first page.
pub(crate) trait CursorVariables {
    /// Replace the `after`/`cursor` variable with `cursor`.
    fn set_cursor(&mut self, cursor: Option<String>);
}

#[cfg(test)]
mod tests;

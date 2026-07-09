//! Pagination helpers for the GraphQL client.

use super::{GraphQLClient, Query};
use crate::VkError;
use crate::api::CursorVariables;
use crate::boxed::BoxedStr;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::borrow::Cow;

const MAX_PAGES: usize = 1000;

impl GraphQLClient {
    /// Fetch and concatenate all pages from a cursor-based connection.
    ///
    /// `query` and `vars` define the base request. The `map` closure
    /// extracts the items and pagination info from each page's response.
    ///
    /// Pagination stops after 1000 pages to avoid infinite loops when cursors
    /// repeat or the API misbehaves.
    ///
    /// # Examples
    /// Borrowed and owned cursors both avoid allocations until needed.
    ///
    /// ```no_run
    /// use std::borrow::Cow;
    /// use serde_json::Map;
    /// use vk::{api::GraphQLClient, PageInfo, VkError};
    ///
    /// # async fn run(client: GraphQLClient) -> Result<(), VkError> {
    /// let vars = Map::new();
    /// client
    ///     .paginate_all::<(), _, serde_json::Value>(
    ///         "query",
    ///         vars.clone(),
    ///         Some(Cow::Borrowed("c1")),
    ///         |_page| Ok((Vec::new(), PageInfo::default())),
    ///     )
    ///     .await?;
    /// let owned = String::from("c2");
    /// client
    ///     .paginate_all::<(), _, serde_json::Value>(
    ///         "query",
    ///         vars,
    ///         Some(Cow::Owned(owned)),
    ///         |_page| Ok((Vec::new(), PageInfo::default())),
    ///     )
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Propagates any [`VkError`] returned by the underlying request or mapper
    /// closure.
    pub async fn paginate_all<Item, Mapper, Page>(
        &self,
        query: impl Into<Query>,
        vars: Map<String, Value>,
        start_cursor: Option<Cow<'_, str>>,
        mut map: Mapper,
    ) -> Result<Vec<Item>, VkError>
    where
        Mapper: FnMut(Page) -> Result<(Vec<Item>, crate::PageInfo), VkError>,
        Page: DeserializeOwned,
    {
        let query = query.into();
        let mut items = Vec::new();
        let mut cursor = start_cursor;
        let mut pages_seen = 0usize;
        loop {
            pages_seen += 1;
            if pages_seen > MAX_PAGES {
                return Err(VkError::BadResponse(
                    format!("pagination exceeded max pages {MAX_PAGES}").boxed(),
                ));
            }
            let data = self
                .fetch_page::<Page, _>(query.clone(), cursor.take(), &vars)
                .await?;
            let (mut page, info) = map(data)?;
            items.append(&mut page);
            if let Some(next) = info.next_cursor()? {
                cursor = Some(Cow::Owned(next.to_string()));
            } else {
                break;
            }
        }
        Ok(items)
    }

    /// Fetch and concatenate all pages of a `graphql_client` codegen'd
    /// operation.
    ///
    /// This is the typed counterpart to [`paginate_all`](Self::paginate_all):
    /// `variables` supplies the base request, the [`CursorVariables`] impl
    /// advances the cursor between pages, and the `map` closure extracts the
    /// items and [`crate::PageInfo`] from each page's `ResponseData`.
    ///
    /// Pagination stops after 1000 pages to avoid infinite loops when cursors
    /// repeat or the API misbehaves. As with `paginate_all`, any items fetched
    /// before an error are discarded: an error from the request or the `map`
    /// closure aborts the whole traversal and yields only that error.
    ///
    /// # Errors
    ///
    /// Propagates any [`VkError`] returned by the underlying request or the
    /// `map` closure, and returns [`VkError::BadResponse`] if the page cap is
    /// exceeded.
    ///
    /// Exercised only from tests during the pilot; its first non-test caller
    /// arrives when a paginated operation migrates, at which point the `expect`
    /// is removed.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the codegen'd paginated ops decode via paginate_operation_as"
        )
    )]
    pub(crate) async fn paginate_operation<Q, Item, Mapper>(
        &self,
        variables: Q::Variables,
        start_cursor: Option<String>,
        map: Mapper,
    ) -> Result<Vec<Item>, VkError>
    where
        Q: graphql_client::GraphQLQuery,
        Q::Variables: CursorVariables + Clone,
        Mapper: FnMut(Q::ResponseData) -> Result<(Vec<Item>, crate::PageInfo), VkError>,
    {
        self.paginate_operation_as::<Q, Q::ResponseData, Item, Mapper>(variables, start_cursor, map)
            .await
    }

    /// Fetch and concatenate all pages of a codegen'd operation, decoding each
    /// page into `T` rather than the generated `ResponseData`.
    ///
    /// This is the paginated counterpart to
    /// [`run_operation_as`](Self::run_operation_as): the query is built from `Q`
    /// (so the selection is schema-checked at compile time) and the cursor is
    /// advanced through the [`CursorVariables`] impl, while each page body is
    /// deserialized into the hand-written domain struct `T`. It is used by the
    /// paginated operations whose generated `ResponseData` would be stricter
    /// than the documented behaviour the tests and fixtures depend on (the
    /// review-thread listing, per-thread comment paging, and the reviews
    /// listing).
    ///
    /// Pagination stops after 1000 pages to avoid infinite loops when cursors
    /// repeat or the API misbehaves. As with `paginate_all`, any items fetched
    /// before an error are discarded.
    ///
    /// # Errors
    ///
    /// Propagates any [`VkError`] returned by the underlying request or the
    /// `map` closure, and returns [`VkError::BadResponse`] if the page cap is
    /// exceeded.
    pub(crate) async fn paginate_operation_as<Q, T, Item, Mapper>(
        &self,
        variables: Q::Variables,
        start_cursor: Option<String>,
        mut map: Mapper,
    ) -> Result<Vec<Item>, VkError>
    where
        Q: graphql_client::GraphQLQuery,
        Q::Variables: CursorVariables + Clone,
        T: DeserializeOwned,
        Mapper: FnMut(T) -> Result<(Vec<Item>, crate::PageInfo), VkError>,
    {
        let mut items = Vec::new();
        let mut cursor = start_cursor;
        let mut pages_seen = 0usize;
        loop {
            pages_seen += 1;
            if pages_seen > MAX_PAGES {
                return Err(VkError::BadResponse(
                    format!("pagination exceeded max pages {MAX_PAGES}").boxed(),
                ));
            }
            let mut vars = variables.clone();
            vars.set_cursor(cursor.take());
            let data = self.run_operation_as::<Q, T>(vars).await?;
            let (mut page, info) = map(data)?;
            items.append(&mut page);
            if let Some(next) = info.next_cursor()? {
                cursor = Some(next.to_string());
            } else {
                break;
            }
        }
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::CursorVariables;
    use crate::test_utils::start_server;
    use graphql_client::GraphQLQuery;

    /// Minimal paginated operation used to exercise [`CursorVariables`] and
    /// [`GraphQLClient::paginate_operation`] against the scripted stub server.
    #[derive(GraphQLQuery)]
    #[graphql(
        schema_path = "graphql/schema.docs.graphql",
        query_path = "graphql/test_pagination.graphql",
        variables_derives = "Clone",
        response_derives = "Debug, Clone, PartialEq"
    )]
    struct PageTestQuery;

    impl CursorVariables for page_test_query::Variables {
        fn set_cursor(&mut self, cursor: Option<String>) {
            self.cursor = cursor;
        }
    }

    /// Extract repository names and pagination info from one page.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "paginate_operation mappers must return Result"
    )]
    fn map_page(
        data: page_test_query::ResponseData,
    ) -> Result<(Vec<String>, crate::PageInfo), VkError> {
        let repositories = data.viewer.repositories;
        let names = repositories
            .nodes
            .into_iter()
            .flatten()
            .flatten()
            .map(|node| node.name)
            .collect();
        let page = repositories.page_info;
        Ok((
            names,
            crate::PageInfo {
                has_next_page: page.has_next_page,
                end_cursor: page.end_cursor,
            },
        ))
    }

    /// Build a page body with one repository and the given pagination cursor.
    fn page_body(name: &str, has_next: bool, end_cursor: Option<&str>) -> String {
        serde_json::json!({
            "data": {
                "viewer": {
                    "repositories": {
                        "nodes": [{ "name": name }],
                        "pageInfo": {
                            "hasNextPage": has_next,
                            "endCursor": end_cursor,
                        }
                    }
                }
            }
        })
        .to_string()
    }

    #[test]
    fn set_cursor_replaces_the_cursor_field() {
        let mut vars = page_test_query::Variables { cursor: None };
        vars.set_cursor(Some("after-1".to_string()));
        assert_eq!(vars.cursor.as_deref(), Some("after-1"));
        vars.set_cursor(None);
        assert_eq!(vars.cursor, None);
    }

    #[tokio::test]
    async fn paginate_operation_concatenates_all_pages() {
        let server = start_server(vec![
            page_body("a", true, Some("c1")),
            page_body("b", false, None),
        ]);
        let items = server
            .client
            .paginate_operation::<PageTestQuery, String, _>(
                page_test_query::Variables { cursor: None },
                None,
                map_page,
            )
            .await
            .expect("pagination succeeds");
        assert_eq!(items, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(server.hits.load(std::sync::atomic::Ordering::SeqCst), 2);
        server.join.abort();
        let _ = server.join.await;
    }

    #[tokio::test]
    async fn paginate_operation_discards_items_before_an_error() {
        let error_body = serde_json::json!({
            "errors": [{ "message": "boom" }]
        })
        .to_string();
        let server = start_server(vec![page_body("a", true, Some("c1")), error_body]);
        let result = server
            .client
            .paginate_operation::<PageTestQuery, String, _>(
                page_test_query::Variables { cursor: None },
                None,
                map_page,
            )
            .await;
        assert!(
            matches!(result, Err(VkError::ApiErrors(_))),
            "expected the second page's GraphQL error to abort the traversal, got {result:?}"
        );
        server.join.abort();
        let _ = server.join.await;
    }
}

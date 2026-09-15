//! Pagination helpers for the GraphQL client.

use super::GraphQLClient;
use super::metrics::record_page_limit;
use crate::VkError;
use crate::api::CursorVariables;
use crate::boxed::BoxedStr;
use serde::de::DeserializeOwned;

/// Maximum number of pages fetched by one pagination operation.
const MAX_PAGES: usize = 1000;

impl GraphQLClient {
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
    /// repeat or the API misbehaves. As with
    /// this method, any items fetched before an error are discarded.
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
                record_page_limit();
                return Err(VkError::BadResponse(
                    format!("pagination exceeded max pages {MAX_PAGES}").boxed(),
                ));
            }
            let mut vars = variables.clone();
            let request_cursor = cursor.take();
            vars.set_cursor(request_cursor.clone());
            let data = self.run_operation_as::<Q, T>(vars).await?;
            let (mut page, info) = map(data)?;
            items.append(&mut page);
            if let Some(next) = info.next_cursor()? {
                if request_cursor.as_deref() == Some(next) {
                    return Err(VkError::BadResponse(
                        "non-progressing pagination (repeated endCursor)".boxed(),
                    ));
                }
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
    /// [`GraphQLClient::paginate_operation_as`] against the scripted stub server.
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
            .paginate_operation_as::<PageTestQuery, page_test_query::ResponseData, String, _>(
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

    /// The cursor characterization: the cursor
    /// supplied to the paginator must land in the request's
    /// `variables.cursor`, overwriting any stale value already present in the
    /// base variables.
    #[rstest::rstest]
    #[case::no_prior_cursor(None)]
    #[case::overwrites_stale_cursor(Some("stale"))]
    #[tokio::test]
    async fn paginate_operation_sends_cursor_in_request_variables(#[case] prior: Option<&str>) {
        let (client, captured, join) = super::super::tests::mock_server_with_capture();
        let _ = client
            .paginate_operation_as::<PageTestQuery, serde_json::Value, String, _>(
                page_test_query::Variables {
                    cursor: prior.map(ToOwned::to_owned),
                },
                Some("fresh".to_string()),
                |_page| {
                    Ok((
                        Vec::new(),
                        crate::PageInfo {
                            has_next_page: false,
                            end_cursor: None,
                        },
                    ))
                },
            )
            .await
            .expect("single page");
        join.abort();
        let _ = join.await;
        super::super::tests::assert_cursor_in_request(&captured, "fresh");
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
            .paginate_operation_as::<PageTestQuery, page_test_query::ResponseData, String, _>(
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

    #[tokio::test]
    async fn paginate_operation_stops_at_the_page_limit() {
        let pages = (0..MAX_PAGES)
            .map(|page| page_body("item", true, Some(&format!("next-{page}"))))
            .collect();
        let server = start_server(pages);

        let result = server
            .client
            .paginate_operation_as::<PageTestQuery, page_test_query::ResponseData, String, _>(
                page_test_query::Variables { cursor: None },
                None,
                map_page,
            )
            .await;

        assert!(
            matches!(result, Err(VkError::BadResponse(message)) if message.contains("max pages"))
        );
        assert_eq!(
            server.hits.load(std::sync::atomic::Ordering::SeqCst),
            MAX_PAGES
        );
        server.join.abort();
        let _ = server.join.await;
    }

    #[tokio::test]
    async fn paginate_operation_stops_on_the_first_repeated_cursor() {
        let server = start_server(vec![
            page_body("first", true, Some("cursor")),
            page_body("second", true, Some("cursor")),
        ]);

        let result = server
            .client
            .paginate_operation_as::<PageTestQuery, page_test_query::ResponseData, String, _>(
                page_test_query::Variables { cursor: None },
                None,
                map_page,
            )
            .await;

        assert!(
            matches!(result, Err(VkError::BadResponse(message)) if message.as_ref() == "non-progressing pagination (repeated endCursor)")
        );
        assert_eq!(server.hits.load(std::sync::atomic::Ordering::SeqCst), 2);
        server.join.abort();
        let _ = server.join.await;
    }
}

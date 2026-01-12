//! Pagination helpers for the GraphQL client.

use super::{GraphQLClient, Query};
use crate::VkError;
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
}

//! Shared deserializers for tolerant GraphQL wire envelopes.
//!
//! GitHub models connection `nodes` as nullable lists of nullable values.
//! Hand-written envelopes use this helper wherever a missing or null list has
//! the established meaning of an empty list, and null entries are discarded.

use serde::Deserialize;
use serde::Deserializer;

/// Decode a nullable GraphQL node list into its non-null items.
pub(crate) fn nullable_nodes<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<Option<T>>>::deserialize(deserializer)?
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect())
}

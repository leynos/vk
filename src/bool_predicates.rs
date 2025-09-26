//! Serde predicates for boolean CLI flags.

/// Returns `true` when the provided flag is `false`.
///
/// Serde uses this helper in `skip_serializing_if` attributes, so the predicate must accept `&bool`.
///
/// # Examples
///
/// ```rust,ignore
/// use crate::bool_predicates;
///
/// assert!(bool_predicates::not(&false));
/// assert!(!bool_predicates::not(&true));
/// ```
#[must_use]
pub fn not<T>(value: &T) -> bool
where
    T: Copy + std::ops::Not<Output = bool>,
{
    !*value
}

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
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "Serde skip_serializing_if requires &bool signature."
)]
#[must_use]
pub(crate) fn not(value: &bool) -> bool {
    !*value
}

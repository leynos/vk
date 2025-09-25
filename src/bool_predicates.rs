//! Serde predicates for boolean CLI flags.

use std::ops::Not as _;

/// Returns `true` when the provided flag is `false`.
///
/// # Examples
///
/// ```
/// use vk::bool_predicates;
///
/// assert!(bool_predicates::not(&false));
/// assert!(!bool_predicates::not(&true));
/// ```
// Serde skip_serializing_if expects an `&bool` predicate.
#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "Serde `skip_serializing_if` expects an `&bool` predicate"
)]
#[must_use]
pub fn not(value: &bool) -> bool {
    value.not()
}

//! Rust type aliases for GitHub's GraphQL custom scalars.
//!
//! `graphql_client` codegen resolves each custom scalar named in a `.graphql`
//! document to a Rust type of the same name that must be in scope of the
//! `#[derive(GraphQLQuery)]`. GitHub's schema declares several scalars (for
//! example `DateTime`, `URI`, and `HTML`) that have no built-in mapping, so a
//! single shared module supplies the aliases and every operation module that
//! needs them writes `use crate::api::scalars::*;`.
//!
//! Keep this module the sole home for scalar aliases: adding a new operation
//! that references an as-yet-unmapped scalar surfaces as a compile error naming
//! the missing type, and the fix is to add the alias here rather than in the
//! consumer.

// `DateTime` is consumed by the reviews operation (`submittedAt`) and `URI` by
// the review-thread operations (the comment `url` field). `HTML` is not yet
// referenced by any migrated operation, so `expect` keeps it in place while
// flagging the moment it becomes used — at which point the attribute is removed
// rather than left as a blanket `allow`.

/// GitHub's `DateTime` scalar: an ISO-8601 instant in UTC.
pub(crate) type DateTime = chrono::DateTime<chrono::Utc>;

/// GitHub's `URI` scalar: an absolute URI transported as a string.
#[expect(
    clippy::upper_case_acronyms,
    reason = "the alias must match the schema scalar name exactly for codegen"
)]
pub(crate) type URI = String;

/// GitHub's `HTML` scalar: a pre-rendered HTML fragment transported as a
/// string.
#[expect(dead_code, reason = "consumed once HTML-bearing operations migrate")]
#[expect(
    clippy::upper_case_acronyms,
    reason = "the alias must match the schema scalar name exactly for codegen"
)]
pub(crate) type HTML = String;

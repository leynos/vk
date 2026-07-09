//! GraphQL client utilities and pagination helpers.
//!
//! The API module exposes a [`GraphQLClient`] for issuing typed
//! `graphql_client` operations, with cursor pagination driven through the
//! [`CursorVariables`] trait.

mod client;
mod pagination;
mod retry;
pub(crate) mod scalars;

pub use client::{Endpoint, GraphQLClient, Token};
pub(crate) use pagination::CursorVariables;
pub use retry::RetryConfig;

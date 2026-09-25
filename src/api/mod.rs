//! GraphQL client utilities and pagination helpers.
//!
//! The API module exposes a [`GraphQLClient`] for issuing typed
//! `graphql_client` operations, with cursor pagination driven through the
//! [`CursorVariables`] trait.

mod client;
pub(crate) mod deserialize;
mod pagination;
mod retry;
pub(crate) mod scalars;

pub use client::{Endpoint, GraphQLClient, Token};
#[cfg(test)]
pub(crate) use pagination::MAX_PAGES;
pub(crate) use pagination::{
    CursorHistory, CursorVariables, page_limit_error, page_limit_exceeded,
};
pub use retry::RetryConfig;

//! GraphQL client utilities and pagination helpers.
//!
//! The API module exposes a [`GraphQLClient`] for issuing requests and a
//! [`paginate`] helper for cursor-based connections.

mod client;
mod pagination;
mod retry;
pub(crate) mod scalars;

pub use client::{Endpoint, GraphQLClient, Query, Token};
pub(crate) use pagination::CursorVariables;
pub use pagination::paginate;
pub use retry::RetryConfig;

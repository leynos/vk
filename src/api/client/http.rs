//! HTTP response wrapper used by the GraphQL client.

/// The status and body returned by a GraphQL HTTP request.
#[derive(Debug)]
pub(super) struct HttpResponse {
    /// The HTTP response status code.
    pub(super) status: u16,
    /// The response body as text.
    pub(super) body: String,
}

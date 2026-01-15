//! HTTP response wrapper used by the GraphQL client.

#[derive(Debug)]
pub(super) struct HttpResponse {
    pub(super) status: u16,
    pub(super) body: String,
}

# GraphQL transport migration guide

The GraphQL client now sends requests through a direct Hyper and rustls
transport. Deployments must account for the resulting networking boundary when
updating `vk`.

## Configure direct endpoint access

The GraphQL transport does not honour the `HTTP_PROXY` or `HTTPS_PROXY`
environment variables. It also does not follow HTTP redirects. Deployments must
therefore configure a directly reachable GraphQL endpoint, including when
overriding the default endpoint through `GITHUB_GRAPHQL_URL`.

These transport boundaries are the implemented second phase of the GitHub API
client modernization recorded in
[ADR 001](adr-001-github-api-client-modernisation.md).

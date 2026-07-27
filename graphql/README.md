# GraphQL schema and query documents

This directory holds the vendored GitHub GraphQL schema and the query documents
that `graphql_client` codegen validates against it at compile time (see
[ADR 001](../docs/adr-001-github-api-client-modernisation.md)).

## Contents

- `schema.docs.graphql` — GitHub's published public schema (free, pro,
  and team plans). This is third-party generated data, not project source; do
  not edit it by hand.
- `*.graphql` — one document per operation group. Each document is named
  after the operation(s) it contains and is referenced by a
  `#[derive(GraphQLQuery)]` item in `src/`.

## Refreshing the schema

Download the current published schema and re-run the test suite; any query the
new schema no longer satisfies fails the build:

    curl -L https://docs.github.com/public/fpt/schema.docs.graphql \
      -o graphql/schema.docs.graphql
    make lint test

Record the refresh (date and reason) in the commit message. GitHub evolves the
schema additively, so refreshes are expected to be safe; a build failure after
a refresh means GitHub removed or renamed something a query relies on.

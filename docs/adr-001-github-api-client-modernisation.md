# Architectural decision record (ADR) 001: GitHub API client modernization

## Status

Accepted (2026-07-09). The project owner accepted the three-part programme
after reviewing the ExecPlan draft: octocrab for the REST resolve path, a
direct hyper transport inside the bespoke GraphQL client (removing reqwest), and
`graphql_client` codegen for compile-time-checked GraphQL queries.

## Date

2026-07-09.

## Context and problem statement

`vk` is a command-line tool that shows unresolved GitHub pull request review
comments. It talks to GitHub through two bespoke, hand-rolled clients built
directly on the `reqwest` crate:

- A GraphQL client (`src/api/client/`) used by every subcommand. It carries a
  substantial observability investment: transcript recording of each request,
  redacted error snippets in failure context, `backon` jittered-exponential
  retry with transient-error classification (HTTP 5xx, HTTP 429, and
  HTML-looking bodies), `serde_path_to_error` deserialization diagnostics, and
  an environment-variable endpoint override (`GITHUB_GRAPHQL_URL`).
- A feature-gated REST client (`src/resolve/rest.rs`, compiled only under the
  `unstable-rest-resolve` feature) that posts review-comment replies with no
  retry and its own environment-variable base-URL override (`GITHUB_API_URL`).

The original decision to hand-roll these clients was never recorded, so the
constraints that justified it are no longer legible to maintainers. The current
arrangement carries four problems:

- Two duplicated header-building and client-construction code paths that drift
  independently.
- No compile-time checking of GraphQL query strings: queries are raw `&str`
  constants and operation names are recovered by string-sniffing the query text.
- A dependency on `reqwest` for work that a single hyper stack could serve,
  when the rest of the intended stack (hyper, rustls) is already present.
- Bespoke REST plumbing (authentication, base-URI handling, header
  construction) that duplicates what a maintained library already provides.

The question this record settles is how to modernize GitHub API access without
regressing the observable behaviour that the test suite pins: command output,
error-message fragments, retry semantics, transcript format, authentication
precedence, and the environment-variable endpoint overrides.

## Decision drivers

- Preserve the observability machinery. Transcript recording, error snippets,
  and retry classification all depend on access to the raw HTTP response
  (status and body), including partial-success GraphQL payloads that carry both
  `data` and `errors`.
- Preserve the test infrastructure. The environment-variable endpoint overrides
  redirect the binary to loopback servers, and roughly twenty tests assert
  exact error text; neither may change.
- Converge on a single HTTP stack rather than maintaining two.
- Gain compile-time validation of GraphQL queries and their variables.
- Minimize bespoke plumbing by delegating maintained concerns to a library
  where doing so does not compromise observability.
- Keep TLS rustls-only; introduce no native-tls or OpenSSL dependency.
- Remain compatible with the minimum supported Rust version (MSRV) of 1.89.

## Options considered

### Option 1: keep both bespoke clients (status quo)

Retain the two `reqwest`-based clients unchanged. This preserves all behaviour
at zero migration cost but resolves none of the problems: there is no
compile-time query checking, the duplicated plumbing persists, and two HTTP
paths remain.

### Option 2: route all traffic through octocrab

Adopt [octocrab](https://docs.rs/octocrab) as the single transport for both
REST and GraphQL. Rejected: octocrab's `graphql()` helper hides the raw
response, discarding the partial-success `data` and the response body that the
transcript, error-snippet, and retry-classification machinery depend on. Its
default builder accepts no custom middleware layers through which that
machinery could be reattached, GraphQL cursor pagination remains hand-rolled
regardless, and the escape-hatch `_post` method would become the primary
interface. The compile-time-checking gain is marginal because octocrab does not
type GraphQL queries itself.

### Option 3: graphql_client codegen with reqwest retained

Adopt `graphql_client` codegen for typed queries while keeping `reqwest` as the
transport. This gains typed queries but keeps two HTTP stacks once octocrab
arrives for the REST path, and retains the bespoke REST plumbing that a
maintained library would otherwise absorb.

### Option 4 (adopted): three-part split

Separate the concerns by transport:

- octocrab (tilde-pinned `~0.54`) serves the REST resolve path only, replacing
  the bespoke `reqwest` REST client and inheriting maintained authentication
  and base-URI plumbing.
- A direct hyper transport (`hyper-util` legacy client plus `hyper-rustls`,
  promoted from dev-dependencies to runtime dependencies) replaces `reqwest`
  inside the bespoke GraphQL client, which retains its observability machinery.
  `reqwest` then leaves the dependency graph.
- `graphql_client` 0.16 codegen validates every query against GitHub's vendored
  public schema
  ([`schema.docs.graphql`](https://docs.github.com/public/fpt/schema.docs.graphql))
  at compile time. Generated types stay private behind conversions to the
  existing exported domain structs.

| Dimension                     | Option 1 | Option 2 | Option 3 | Option 4 |
| ----------------------------- | -------- | -------- | -------- | -------- |
| Compile-time query checking   | No       | Marginal | Yes      | Yes      |
| HTTP stacks after change      | Two      | One      | Two      | One      |
| Observability preserved       | Yes      | No       | Yes      | Yes      |
| Bespoke REST plumbing removed | No       | Yes      | No       | Yes      |
| Maintained REST library       | No       | Yes      | Yes      | Yes      |

_Table 1: Comparison of the four GitHub API client options against the decision
drivers._

## Decision outcome

Adopt option 4, the three-part split. octocrab's value concentrates in its
typed REST surface and maintained plumbing, whereas the bespoke GraphQL
client's value is its observability, which octocrab's `graphql()` helper would
discard. octocrab's own GraphQL example delegates query typing to
`graphql_client`, confirming that the intended division of labour matches the
tools' strengths.

The programme is delivered as three pull requests, each behaviour-preserving
and gated by the existing test suite:

1. Adopt octocrab for the REST resolve path.
2. Replace `reqwest` inside the GraphQL client with a hyper transport and
   remove `reqwest` from the dependency graph.
3. Adopt `graphql_client` codegen so malformed queries fail the build rather
   than a runtime request.

The REST-first ordering proves octocrab in-tree with the smallest blast radius;
the transport change then removes `reqwest` before the largest change; the
codegen change is type-level only and benefits from a settled transport beneath
it.

## Migration plan

The migration is tracked as a living document in the ExecPlan
[`docs/execplans/adopt-octocrab.md`](execplans/adopt-octocrab.md). Its numbered
phases correspond to the three pull requests above, and it records the
constraints, tolerances, risks, decision log, and per-phase acceptance criteria
in detail. This record does not duplicate that content; consult the ExecPlan
for the authoritative migration sequence and progress.

## Known risks and limitations

- octocrab has shipped breaking changes in patch releases (upstream issue 899).
  The dependency is therefore tilde-pinned (`~0.54`) rather than caret-ranged,
  with the reason recorded both here and in a `Cargo.toml` comment; widen only
  after review.
- The hyper transport must reproduce the pooling, TLS root-store, and
  total-request timeout semantics that `reqwest` provided for free. A subtle
  difference under failure could change behaviour; the retry and timeout unit
  tests act as a characterization harness because the transport change alters
  nothing else.
- GitHub's vendored public schema is roughly 1.5 MB, and each
  `#[derive(GraphQLQuery)]` re-parses it at compile time. The build-time impact
  is bounded by the tolerance recorded in the ExecPlan; operations are grouped
  per document to amortize the parse.
- GitHub's custom scalars (`DateTime`, `URI`, `HTML`, and related types)
  require Rust type aliases in scope of each derive. A shared `scalars` module
  supplies them; a missing alias surfaces as an easily misread compile error.

## Design for reuse

The refitted GraphQL client is shaped for cheap future extraction into a shared
crate. The sibling project frankie (a code-review terminal user interface,
currently REST-only on octocrab) will need the same GraphQL review-thread
surface, and the executor (transport, retry, transcript, typed operations, and
cursor pagination) is the reusable part while query documents stay per-project.
To keep extraction cheap, `vk`-specific coupling (`VkError` and
`vk::environment`) is confined to the edges of the transport and typed modules
rather than woven through them. Extraction itself is out of scope for this
decision.

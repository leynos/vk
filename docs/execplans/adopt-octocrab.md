# Octocrab REST migration (PR 1): hyper transport and typed GraphQL roadmap

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`,
and `Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: IN PROGRESS

## Purpose / big picture

`vk` is a command-line tool that shows unresolved GitHub pull request review
comments in the terminal. Its bespoke GraphQL client (`src/api/client/`) still
uses `reqwest` and is used by every subcommand. When the
`unstable-rest-resolve` feature is enabled, the REST reply path in
`src/resolve/rest.rs` uses Octocrab's raw `_post` route. Octocrab supplies the
authentication and base-URI plumbing; the builder adds
`x-github-api-version: 2022-11-28` and `Accept: application/vnd.github+json`.
`post_reply` wraps the complete request in the total deadline, while Octocrab
also receives the connect, read, and write timeout settings. A 404 warns and
continues to GraphQL resolution; success does the same, while another non-2xx
response aborts with the reply route and status in the diagnostic.

An earlier draft of this plan proposed routing everything through
[octocrab](https://docs.rs/octocrab). Review concluded octocrab is a weak fit
for the GraphQL side (its `graphql()` helper hides the raw response that `vk`'s
transcript, error-snippet, and retry machinery depend on) but a genuine win for
the REST side (typed pull-request APIs, maintained auth and base-URI plumbing).
The revised programme is therefore three separable changes, delivered as three
pull requests:

1. PR 1 — adopt octocrab for the REST resolve path.
2. PR 2 — replace `reqwest` inside the bespoke GraphQL client with a direct
   hyper transport, then remove `reqwest` from the dependency graph. The binary
   converges on one HTTP stack (hyper/rustls), shared with octocrab.
3. PR 3 — adopt `graphql_client` codegen so every GraphQL query and its
   variables are checked against GitHub's published schema at compile time,
   removing the raw query-string constants and hand-maintained response
   envelopes.

`vk`'s externally observable behaviour is preserved throughout: command output,
error messages, retry semantics, transcript recording, authentication
precedence, and the environment-variable endpoint overrides that the entire
test suite depends on.

Success is observable per PR: after PR 1, `tests/resolve.rs` passes with
octocrab serving the reply path; after PR 2, `cargo tree -i reqwest` reports
the crate absent while the full suite passes; after PR 3, malforming any query
in the vendored `.graphql` documents fails the build rather than a runtime
request, and the suite still passes. All gates (`make check-fmt`, `make lint`,
`make test`, `make markdownlint`, `make nixie`) pass at every commit.

## Constraints

Hard invariants that must hold throughout implementation. Violation requires
escalation, not workarounds.

- The public CLI behaviour must not change: identical stdout/stderr for the
  same inputs, including error-message fragments asserted by tests (for example
  `"status 500"`, `"body snippet:"`, and serde error paths such as
  `"repository.pullRequest.reviewThreads"`).
- The environment-variable overrides `GITHUB_GRAPHQL_URL` (full GraphQL
  endpoint URL) and `GITHUB_API_URL` (REST base URL) must keep working, because
  every network-facing test redirects the binary to a loopback server through
  them.
- Token precedence must remain: `--github-token` flag, then
  `VK_GITHUB_TOKEN`, then `GITHUB_TOKEN`, then the config-file value
  (`src/auth.rs`).
- GraphQL requests must keep sending `User-Agent: vk`,
  `Accept: application/vnd.github+json`, and `Authorization: Bearer <token>`
  (only when a token is present; anonymous access must keep working and keep
  printing the "GitHub token not set, using anonymous API access" warning).
- The transcript feature (`VK_TRANSCRIPT` / constructor parameter) must keep
  writing the same JSON-lines format consumed by
  `tests/e2e/common.rs::load_transcript` and the `tests/fixtures/pr42.json`
  fixture: one object per line with keys `operation`, `status`, `request`, and
  `response` (response body truncated to 500 characters).
- Retry behaviour on the GraphQL path must be preserved: jittered
  exponential backoff via `backon` with the transient-error classification in
  `src/api/retry.rs` (retry on request errors, empty responses, HTTP 5xx, HTTP
  429, and HTML-looking bodies). The REST reply path performs no retries today
  and must not gain any.
- TLS must remain rustls-based; do not introduce a native-tls or OpenSSL
  dependency.
- Dependency versions must use caret requirements unless a tilde pin is
  documented with a reason (see Decision Log for the octocrab pin).
- The error type `VkError` and its variants remain the error surface of
  `src/api/`; octocrab's, hyper's, and `graphql_client`'s error types must be
  mapped at module boundaries and never leak into public signatures.
- The exported domain types (`ReviewThread`, `ReviewComment`,
  `CommentConnection`, `PageInfo`, `PullRequestReview`, `Issue`, `User`) remain
  the types consumers and the printer use; generated GraphQL types stay private
  behind a conversion layer.
- All commit gates pass on every commit: `make check-fmt`, `make lint`,
  `make test`, and for documentation changes `make markdownlint` and
  `make nixie`.
- No single source file may exceed 400 lines; module-level `//!` comments
  and en-GB-oxendict spelling are required in all new code. The vendored
  GraphQL schema is third-party generated data, not source, and is exempt.

## Tolerances (exception triggers)

- Scope: if any single PR requires editing more than 20 source files or a
  net change beyond roughly 1,200 lines (excluding the vendored schema and
  `.graphql` documents), stop and escalate.
- Interface: if a public API of the `vk` library crate (anything re-exported
  from `src/lib.rs`) must change signature or be removed, stop and escalate —
  with one pre-approved exception: PR 3 may add typed operation-execution
  methods to `GraphQLClient` and deprecate (not remove) the string-based
  `run_query`/`fetch_page`/`paginate_all` surface if call-site migration leaves
  them unused.
- Dependencies: this plan pre-approves `octocrab` and `http` (PR 1 —
  `http` is already in the tree; `reqwest::header` re-exports it, and octocrab's
  `add_header` and PR 2's transport interface both need it directly),
  promotion of `hyper`, `hyper-util`, `hyper-rustls`, and `http-body-util` from
  dev-dependencies to runtime dependencies (PR 2), and `graphql_client` (PR 3).
  Any further new direct dependency requires escalation.
- Behaviour: if preserving an asserted behaviour (error text, transcript
  format, header set, retry counts) proves impossible, stop and present
  options; do not weaken tests to fit.
- Iterations: if a gate still fails after three fix attempts on the same
  failure, stop and escalate.
- Build time: if PR 3's schema-parsing derives push a clean `cargo build`
  more than 50% above the pre-PR baseline (record the baseline first), stop and
  escalate with options (group operations per derive, prune the schema).

## Risks

- Risk: octocrab's typed REST models may not deserialize the minimal inline
  JSON bodies used by `tests/resolve.rs`, and its error mapping may not
  reproduce the 404-non-fatal / other-non-2xx-fatal semantics exactly.
  Severity: medium. Likelihood: medium. Mitigation: PR 1 starts with a spike;
  test stub bodies may be enriched to realistic fixtures (that is
  strengthening, not weakening); fall back to octocrab's raw `_post` route
  (which inherits auth and base-URI middleware but leaves response handling to
  `vk`) if the typed handler cannot match semantics.
- Risk: octocrab has shipped breaking changes in patch releases before
  (upstream issue 899). Severity: medium. Likelihood: medium. Mitigation: pin
  with a tilde requirement (`~0.54`) and record the reason in a `Cargo.toml`
  comment and the architectural decision record (ADR).
- Risk: a direct hyper transport must reassemble what `reqwest` provided
  for free: connection pooling, TLS configuration, total-request timeout, and
  body collection. A subtle difference (for example timeout scope or TLS root
  store) could change behaviour under failure. Severity: medium. Likelihood:
  medium. Mitigation: the retry and timeout unit tests in
  `src/api/client/tests.rs` count attempts against scripted local servers and
  act as a characterization harness; PR 2 changes nothing but the transport
  internals, so any drift surfaces there. Root-store choice (webpki versus
  native) is recorded in the Decision Log during implementation.
- Risk: `graphql_client`'s generated response types may produce
  `serde_path_to_error` paths that differ from the hand-written structs,
  breaking the e2e assertion on `"repository.pullRequest.reviewThreads"`.
  Severity: low. Likelihood: low (generated fields carry the same camelCase
  serde renames). Mitigation: the e2e test is in the harness; if paths differ,
  adjust the conversion boundary, not the test.
- Risk: GitHub's vendored schema is ~73,000 lines (~1.5 MB) and each
  `#[derive(GraphQLQuery)]` re-parses it at compile time. Severity: low.
  Likelihood: medium. Mitigation: benchmark before and after (see Build time
  tolerance); group operations into shared documents where sensible; generated
  code is small because only selected fields are generated.
- Risk: GitHub's custom scalars (`DateTime`, `URI`, `HTML`, `GitObjectID`)
  need Rust type aliases in scope of each derive; a missed alias is a compile
  error easily misread. Severity: low. Likelihood: high (it will arise).
  Mitigation: a single shared `scalars` module
  (`type DateTime = chrono::DateTime<chrono::Utc>`, `type URI = String`,
  `type HTML = String`) imported by every operation module.
- Risk: `paginate_all` currently injects the cursor into an untyped JSON
  variables map; typed `Variables` structs break that mechanism. Severity:
  medium. Likelihood: certain (by design). Mitigation: PR 3 introduces a small
  `CursorVariables` trait (set the cursor on a typed variables struct)
  implemented per paginated operation; written test-first.
- Risk: binary size and compile time grow while both reqwest and the hyper
  stack are present (during PR 1, and PR 2 before removal). Severity: low.
  Likelihood: certain but transient. Mitigation: PR 2 ends with
  `cargo tree -i reqwest` failing to find the package.

## Progress

- [x] (2026-07-09 12:20Z) Explored codebase (client layer, consumers, test
  infrastructure, docs) and researched octocrab 0.54; findings folded into this
  plan.
- [x] (2026-07-09 12:40Z) ExecPlan drafted and linked from
  `docs/contents.md`.
- [x] (2026-07-09 14:10Z) Scope revised after review: octocrab confined to
  REST; GraphQL keeps the bespoke client on a hyper transport with
  `graphql_client` codegen. `graphql_client` 0.16 researched and confirmed
  transport-agnostic.
- [x] (2026-07-09 15:30Z) Stage A: ADR
  `docs/adr-001-github-api-client-modernisation.md` authored and linked from
  `docs/vk-design.md` and `docs/contents.md`; octocrab `~0.54` added to
  `Cargo.toml` (default features off; `default-client`, `jwt-rust-crypto`,
  `rustls`, `rustls-ring`, `timeout`; `retry` excluded deliberately).
- [x] (2026-07-09 17:20Z) PR 1: octocrab REST resolve path complete.
  `src/resolve/rest.rs` reworked onto octocrab via the raw `_post` route with
  `RestClient::new` and `post_reply` signatures and semantics preserved,
  `github_client` deleted, `x-github-api-version` and `Accept` headers restored
  via builder `add_header` with a direct `http` dependency; `tests/resolve.rs`
  green unchanged; design doc updated; all gates green; CodeRabbit review
  completed with zero findings; draft pull request opened as leynos/vk#194.
- [x] (2026-07-28) Review follow-up restored the total REST reply deadline,
  strengthened raw POST coverage for the route, body, authentication, required
  headers, and status handling, and reconciled the documentation findings.
- [x] (2026-08-03) Documentation review follow-up corrected the dependency
  record, documentation index, Oxford spelling, and REST ownership guidance.
- [x] (2026-07-09 18:30Z) PR 2 complete: `src/api/client/transport.rs`
  added with `GraphQLClient` delegating to it; reqwest removed from the graph
  entirely (`cargo tree -i reqwest` finds nothing in normal, dev, and
  all-features graphs); transcript-replay test `e2e_pr_42` passes; design doc
  and e2e testing guide corrected; all gates green; CodeRabbit review completed
  with zero findings; draft pull request opened as leynos/vk#195 (stacked on PR
  1).
- [ ] PR 3: vendored schema, `.graphql` documents, generated types behind a
  conversion layer, typed pagination; raw query constants deleted; full suite
  green.
- [ ] Documentation pass per PR (`docs/vk-design.md` and the e2e guide
  correction in whichever PR touches it first); retrospective completed.

## Surprises & discoveries

- Observation: despite the branch name, no octocrab groundwork exists; this
  is a from-scratch adoption. Evidence: `grep octocrab Cargo.toml src/ -r`
  returns nothing. Impact: the plan treats the migration as new work, not a
  continuation.
- Observation: `docs/vk-end-to-end-testing-guide.md` describes `third-wheel`
  as a man-in-the-middle proxy, but the implementation only uses
  `third_wheel::hyper` re-exports for a plain loopback stub server driven by
  env-var endpoint overrides. Evidence: `tests/utils/mod.rs:186-192` sets
  `GITHUB_GRAPHQL_URL` and `GITHUB_API_URL`; no proxy or certificate trust is
  configured anywhere. Impact: only the env-var overrides need preserving; the
  guide should be corrected when first touched.
- Observation: the transcript writes the raw, unredacted request payload;
  redaction (`redact_sensitive`) applies only to error-context snippets.
  Evidence: `src/api/client/transcript.rs` versus
  `src/api/client/mod.rs:126-134`. Impact: parity is the goal; do not silently
  change redaction behaviour during the migration. Flagged for a possible
  follow-up outside this plan.
- Observation: octocrab 0.54 with `default-features = false` fails to
  compile unless one of its JWT crypto features is enabled, even when app (JWT)
  auth is unused. Evidence: `compile_error!` at octocrab's `src/lib.rs:304`
  during the first gate run. Impact: the dependency carries `jwt-rust-crypto`
  (pure-Rust, matching the ring/rustls posture) alongside `default-client`,
  `rustls`, `rustls-ring`, and `timeout`; this is the final feature set
  anticipated by the Interfaces section.
- Observation: octocrab's own GraphQL example delegates query typing to
  `graphql_client`, confirming the two tools' division of labour matches this
  plan's (octocrab does not provide typed GraphQL itself). Evidence:
  `examples/graphql_issues.rs` in the octocrab repository. Impact: supports
  confining octocrab to REST.

## Decision log

- Decision: adopt the three-part programme — octocrab for REST only, a
  direct hyper transport for the bespoke GraphQL client, and `graphql_client`
  codegen for typed queries — instead of routing all traffic through octocrab.
  Rationale: the bespoke GraphQL client is heavily leveraged for
  instrumentability (transcript recording, body-snippet error context,
  transient-retry classification on raw bodies), all of which octocrab's
  `graphql()` helper hides; octocrab's value concentrates in its typed REST
  surface and maintained plumbing. This keeps the observability investment,
  converges on one HTTP stack, and adds the compile-time query checking the
  bespoke client always lacked. Date/Author: 2026-07-09, direction given by the
  user in review of the first draft.
- Decision: deliver as three pull requests in the order REST (PR 1),
  transport (PR 2), codegen (PR 3). Rationale: PR 1 is the smallest and proves
  octocrab in-tree with minimal blast radius; PR 2 removes reqwest early so the
  dependency win lands before the largest change; PR 3 is type-level only and
  benefits from a settled transport underneath it. PRs 2 and 3 are
  order-independent if circumstances change. Date/Author: 2026-07-09, planning
  session.
- Decision: keep the `GraphQLClient` facade and `VkError` taxonomy
  throughout; all three PRs change internals or add typed surface only.
  Rationale: consumers (`src/commands.rs`, `src/review_threads.rs`,
  `src/reviews.rs`, `src/issues.rs`, `src/branch_pr/`, `src/resolve/`) and
  roughly twenty network tests couple to the facade, the error text, and the
  env-var overrides. Date/Author: 2026-07-09, planning session.
- Decision: keep `backon` retry as the single retry authority on the
  GraphQL path; build octocrab (REST) without its retry feature so the reply
  path stays retry-free as today. Rationale: preserves tested attempt counts;
  octocrab's retry layer cannot see GraphQL rate-limit errors (HTTP 200 with an
  `errors` payload) anyway. Date/Author: 2026-07-09, planning session.
- Decision: pin octocrab with a tilde requirement (`~0.54`). Rationale:
  octocrab has a documented history of breaking changes in patch releases
  (upstream issue 899). AGENTS.md permits tilde pins where the reason is
  documented; this is that documentation, and the ADR repeats it. Date/Author:
  2026-07-09, planning session.
- Decision: in PR 3, keep the exported domain structs and convert from
  generated `ResponseData` types at a private boundary (`From`
  implementations), rather than exporting generated types. Rationale: the
  domain structs are public API (re-exported from `src/lib.rs`) and are
  consumed by the printer and summary modules; the generated types are an
  implementation detail of the wire format. Date/Author: 2026-07-09, planning
  session.
- Decision: design the refitted GraphQL client (PRs 2 and 3) for cheap
  future extraction into a shared crate. Rationale: the sibling project frankie
  (a code-review terminal user interface (TUI), currently REST-only octocrab)
  will need the GraphQL-only review-thread surface (`isResolved`,
  `resolveReviewThread`) that motivated this client, and the executor
  (transport, retry, transcript, typed operations, cursor pagination) is the
  reusable part while query documents stay per-project. Concretely: keep
  `VkError` and `vk::environment` coupling at the edges of the transport and
  typed modules rather than woven through them. Extraction itself is out of
  scope for this plan. Date/Author: 2026-07-09, follow-up review discussion.
- Decision: PR 1 uses octocrab's raw `_post` route for the reply, not the
  typed `pulls(...).comment(id).reply(...)` route. Rationale: the typed route
  funnels non-2xx responses through octocrab's `Error::GitHub`, whose display
  carries neither the request path nor the HTTP status code that
  `tests/resolve.rs` asserts in stderr; `_post` returns the raw
  `http::Response`, keeping the 404-non-fatal / other-non-2xx-fatal mapping and
  error text in `vk`'s hands. Timeout mapping: octocrab's read and write
  timeouts remain configured, `connect_timeout` maps directly, and `post_reply`
  additionally enforces reqwest's original total-request deadline. Date/Author:
  2026-07-09, PR 1 implementation.
- Decision: in PR 1, keep octocrab's own `User-Agent: octocrab` on the
  REST path (no test asserts the REST user agent, and octocrab hard-codes its
  value first in the header list), but restore the
  `x-github-api-version: 2022-11-28` pin and the
  `Accept: application/vnd.github+json` header via the builder's `add_header`,
  using a direct `http` dependency. Rationale: the API-version pin is
  documented behaviour in `docs/vk-design.md` and protects against GitHub
  changing its default API version; dropping it silently would contradict the
  design document. Date/Author: 2026-07-09, PR 1 implementation.
- Decision: record the programme in a new ADR,
  `docs/adr-001-github-api-client-modernisation.md`. Rationale: no ADRs exist;
  the bespoke-client choice was never recorded. AGENTS.md requires substantive
  decisions to be captured as ADRs following
  `docs/documentation-style-guide.md`. Date/Author: 2026-07-09, planning
  session (path renamed from `adr-001-adopt-octocrab.md` when the scope
  changed).

## Outcomes & retrospective

PR 1 is complete. The REST reply path uses Octocrab's raw `_post` route and
preserves the route, headers, total deadline, authentication, and
404-non-fatal/other-non-2xx-fatal semantics. The recorded PR 1 validation ran
`cargo test --features unstable-rest-resolve --test resolve` and the repository
gates; the 2026-07-28 follow-up added total-deadline coverage and strengthened
the route, body, authentication, header, and status assertions.

PR 2 remains future work: replace `reqwest` in the bespoke GraphQL client with
the planned hyper transport and verify its removal from the dependency graph.
PR 3 remains future work: add typed GraphQL code generation and migrate the
query call sites without changing the public domain types.

## Context and orientation

The repository is a single Rust crate (`vk`, edition 2024, minimum supported
Rust version (MSRV) 1.89) with sources under `src/` and integration tests under
`tests/`. The `Makefile` is the canonical command runner. Read `AGENTS.md`
before contributing.

The API layer, all under `src/api/`:

- `src/api/mod.rs` re-exports `GraphQLClient`, `Endpoint`, `Query`, `Token`
  (from `client/`), `paginate` (from `pagination.rs`), and `RetryConfig` (from
  `retry.rs`).
- `src/api/client/mod.rs` defines `GraphQLClient` (fields: a
  `reqwest::Client`, headers, endpoint, optional transcript, and retry config)
  with constructors `new` (endpoint from the `GITHUB_GRAPHQL_URL` env var,
  default `https://api.github.com/graphql`), `with_endpoint`, and
  `with_endpoint_retry`; methods `run_query` (POST, backon retry loop),
  `fetch_page` (cursor merge), and private `execute_single_request` /
  `process_graphql_response` (status handling, transcript logging, GraphQL
  error surfacing, `serde_path_to_error` deserialization).
- `src/api/client/helpers.rs` builds headers (`User-Agent: vk`, `Accept`,
  optional `Authorization`) and provides snippet/redaction helpers.
- `src/api/client/types.rs` defines the `Query`, `Token`, and `Endpoint`
  newtypes and the `GraphQLResponse<T>` envelope.
- `src/api/client/transcript.rs` appends one JSON line per request to the
  optional transcript file.
- `src/api/client/pagination.rs` implements `paginate_all` (cursor loop,
  1,000-page cap); `src/api/pagination.rs` holds the generic `paginate` helper
  and `PageInfo` handling.
- `src/api/retry.rs` defines `RetryConfig` (5 attempts, 200 ms base delay,
  30 s request timeout, jitter) and `should_retry` transient classification.

GraphQL queries are raw string constants in `src/graphql_queries.rs`
(`THREADS_QUERY`, `COMMENT_QUERY`, `ISSUE_QUERY`, `PR_FOR_BRANCH_QUERY`) and in
`src/resolve/graphql.rs` (`RESOLVE_THREAD_MUTATION`, `REVIEW_COMMENTS_PAGE`).
Responses deserialize into per-module serde structs that mirror the GraphQL
wire shape; the exported ones (`ReviewThread`, `ReviewComment`,
`CommentConnection`, `PageInfo`, `PullRequestReview`, `Issue`, `User`) are
public API and survive all three PRs.

The REST reply boundary (only compiled with `--features unstable-rest-resolve`)
uses an Octocrab client with the resolved GitHub API base URL, defaulting to
`https://api.github.com`. Its raw `_post` route posts to
`repos/{owner}/{name}/pulls/{pull_number}/comments/{comment_id}/replies` with
the pinned API-version and JSON `Accept` headers. The complete request is
bounded by the total deadline, in addition to Octocrab's connect, read, and
write timeouts. A 404 is non-fatal (warn and continue), another non-2xx status
is fatal with the route and status retained, and the path has no retry.

Authentication: `src/auth.rs::resolve_github_token` implements the precedence
chain; no `gh` CLI integration exists. The token is a plain string handed to
the client constructors.

Tests that constrain this work:

- `src/api/client/tests.rs` spins up loopback hyper servers and constructs
  clients via `with_endpoint`/`with_endpoint_retry` using endpoints without a
  path; it counts retry attempts and asserts error-text fragments.
- `src/test_utils/test_http.rs` and `src/branch_pr/tests.rs` follow the
  same pattern.
- `tests/utils/mod.rs::vk_cmd` runs the real binary with
  `GITHUB_GRAPHQL_URL=http://{addr}/graphql`, `GITHUB_API_URL=http://{addr}`,
  and `GITHUB_TOKEN=dummy`.
- `tests/auth.rs` asserts the captured `Authorization` header and the
  anonymous-access warning.
- `tests/resolve.rs` asserts the exact REST path and status-code semantics.
- `tests/cli.rs` holds two insta snapshots of rendered output (client-swap
  insensitive, data-shape sensitive).
- `tests/e2e/common.rs::load_transcript` and `tests/fixtures/pr42.json`
  encode the transcript JSON-lines format.

Key library facts (verified 2026-07-09):

- octocrab 0.54.0 (MSRV 1.85) builds on hyper 1.x and tower, rustls with
  the ring provider by default. `OctocrabBuilder` provides `personal_token`,
  `base_uri` (applies to all routes), `add_header`, and timeouts.
  `pulls(owner, repo)` exposes typed review-comment operations including
  replying to a comment; the raw `_post(route, body)` method returns an
  unprocessed `http::Response` as a fallback. Resolving a review thread has no
  REST endpoint; the `resolveReviewThread` GraphQL mutation is the only way.
- `graphql_client` 0.16.0 (January 2026, maintained, MSRV 1.66):
  `#[derive(GraphQLQuery)]` with `schema_path`/`query_path` generates
  `Variables` and `ResponseData` types per operation.
  `Operation::build_query(variables)` returns a `QueryBody` that serializes to
  the standard `{"query", "variables", "operationName"}` envelope, and
  responses are plain serde — both compose with any transport, so the bespoke
  client's envelope handling, transcript, and `serde_path_to_error` diagnostics
  continue to work. The reqwest features are optional and stay off. Custom
  scalars used by GitHub (`DateTime`, `URI`, `HTML`, and friends) need type
  aliases in scope of the derive.
- GitHub's public GraphQL schema is published at
  `https://docs.github.com/public/fpt/schema.docs.graphql` (~73,000 lines, ~1.5
  MB SDL); vendoring it whole is the established practice (octocrab and
  graphql_client both do so in their examples).

## Plan of work

Stage A (lands with PR 1): author
`docs/adr-001-github-api-client-modernisation.md` following the ADR template in
`docs/documentation-style-guide.md`. Status: Accepted. Context: two bespoke
reqwest clients, an undocumented prior decision, and no compile-time query
checking. Options Considered: keep bespoke; route everything through octocrab;
the adopted three-part split; `graphql_client` plus reqwest without octocrab.
Decision Outcome: the three-part programme, with the octocrab tilde pin and its
rationale. Migration Plan: reference this ExecPlan. Reference the ADR from
`docs/vk-design.md` and list it in `docs/contents.md`.

### PR 1 — octocrab for the REST resolve path

Add the dependency (final feature list recorded here after the spike; the
`retry` feature is deliberately excluded so no retry layer exists, matching
today's retry-free reply path):

```toml
# Tilde pin: octocrab has shipped breaking changes in patch releases
# (upstream issue 899); widen only after review.
octocrab = { version = "~0.54", default-features = false, features = [
    "default-client", "jwt-rust-crypto", "rustls", "rustls-ring", "timeout",
] }
```

Rework `src/resolve/rest.rs` behind its existing `pub(crate)` surface:
`RestClient::new(token, api, timeout, connect_timeout)` builds an
`octocrab::Octocrab` via `OctocrabBuilder` (`personal_token` when the token is
non-empty, `base_uri` from the existing parameter → `GITHUB_API_URL` → default
resolution, and connect/read/write timeouts from the existing arguments).
`post_reply` additionally enforces the original total-request deadline and uses
octocrab's raw `_post` route with the hand-built
`/repos/{owner}/{name}/pulls/{pull_number}/comments/{comment_id}/replies`
endpoint. It preserves the semantics `tests/resolve.rs` asserts: the exact
request path, a 404 mapped to a warning and success, and every other non-2xx
response mapped to a fatal `VkError` containing the route and status. Delete
`github_client` and the hand-rolled header constants. Acceptance:
`cargo test --features unstable-rest-resolve` and `make lint` (all-features)
pass; `tests/resolve.rs` unchanged in what it asserts.

### PR 2 — hyper transport; remove reqwest

Add a private transport module `src/api/client/transport.rs` owning a pooled
hyper client (`hyper_util::client::legacy::Client` with a `hyper_rustls` HTTPS
connector) and exposing one method mirroring `execute_single_request`'s
contract: take the JSON payload and headers, POST to the configured endpoint
URL (kept whole — no base/route splitting is needed because the bespoke client
posts to one absolute URL), enforce the total-request timeout via
`tokio::time::timeout` (mirroring reqwest's `.timeout()` scope: connect plus
body), collect the body with `http-body-util`, and return
`HttpResponse { status, body }` or a `VkError::RequestContext` with today's
context text. Promote `hyper`, `hyper-util`, `hyper-rustls`, and
`http-body-util` to runtime dependencies; record the chosen TLS root store
(webpki roots, matching the current `rustls-tls` posture) in the Decision Log.
Switch `execute_single_request` to delegate to the transport; the transcript
call, status check, snippets, retry loop, and headers logic are untouched.
Remove `reqwest` from `Cargo.toml` and replace any residual `reqwest::header`
imports with the `http` crate equivalents (they are the same types
re-exported). Acceptance: `make test` passes without any test edits;
`cargo tree -i reqwest` reports the package is not found.

### PR 3 — typed GraphQL via graphql_client codegen

Record the build-time baseline first (`cargo build` from clean, wall clock).
Vendor the schema at `graphql/schema.docs.graphql` with a short
`graphql/README.md` noting the source URL and refresh procedure. Move each
operation into a `.graphql` document under `graphql/` (thread listing, comment
paging, issue lookup, PR-for-branch, review-comments paging, the
`resolveReviewThread` mutation, and reviews listing), grouping related
operations per file to amortize the per-derive schema parse. Create a shared
scalars module (`type DateTime = chrono::DateTime<chrono::Utc>`,
`type URI = String`, `type HTML = String`, extended as compile errors direct).
Derive `GraphQLQuery` per operation with
`response_derives = "Debug, Clone, PartialEq"`.

Client surface: add a typed method to `GraphQLClient` (see Interfaces) that
takes an operation's `Variables`, builds the envelope with `build_query`, and
reuses the existing POST, transcript, retry, and `serde_path_to_error`
machinery; the codegen'd operation name replaces the string-sniffing
`operation_name` helper on this path. For pagination, introduce a
`CursorVariables` trait (set/replace the cursor on a typed variables struct)
and a typed `paginate_operation` counterpart to `paginate_all`, written
test-first against the existing scripted stub servers.

Conversion boundary: implement `From<ResponseData>` (or dedicated mapper
functions where `From` is awkward) producing the existing exported domain
structs, so `src/commands.rs`, the printer, and the summary modules do not
change. Migrate call sites module by module (review_threads, reviews, issues,
branch_pr, resolve/graphql), deleting each raw query constant as its operation
moves; `src/graphql_queries.rs` is deleted at the end. Red-green applies to the
new units (the `CursorVariables` trait, each conversion): write the rstest
cases first against fixture JSON and observe the expected failure before
implementing. Acceptance: full suite green; introducing a deliberate typo into
any `.graphql` document fails `cargo build` (demonstrate once, then revert);
build-time delta within tolerance.

Documentation lands with each PR: `docs/vk-design.md` networking and resolve
sections (PRs 1–2), the third-wheel correction in
`docs/vk-end-to-end-testing-guide.md` (first PR that touches test docs), the
GraphQL section rewrite and schema-refresh procedure (PR 3), and
`docs/contents.md` whenever a document is added.

## Concrete steps

All commands run from the repository root. Long outputs go through `tee` to a
log file for review, for example:

```shell
make test 2>&1 | tee "/tmp/test-vk-adopt-octocrab.out"
```

Per-milestone sequence (repeat for each commit within each PR):

1. Make the edits described in Plan of work.
2. `make check-fmt` (apply `make fmt` first if needed).
3. `make lint` — expect zero warnings; clippy runs with `-D warnings` and
   `--all-features`, so the REST feature code is always linted.
4. `make test` — expect all tests to pass. The suite currently passes on a
   clean checkout; any new failure is caused by the change under test.
5. For documentation edits: `make markdownlint` and `make nixie`.
6. Commit with an imperative-mood message; update the `Progress` section of
   this plan in the same commit.

Useful focused commands:

```shell
cargo test --features unstable-rest-resolve --test resolve 2>&1 | tee /tmp/test-vk-adopt-octocrab.out
cargo test api::client 2>&1 | tee /tmp/test-vk-adopt-octocrab.out
cargo test --test e2e -- --ignored e2e_pr_42 2>&1 | tee /tmp/test-vk-adopt-octocrab.out
cargo tree -i reqwest        # PR 2 exit: expect "package … not found"
cargo tree -d                # check duplicate transitive versions
curl -L https://docs.github.com/public/fpt/schema.docs.graphql -o graphql/schema.docs.graphql
```

Expected shape of a passing test run (counts will drift as tests are added):

```text
test result: ok. 0 failed; finished in …
```

## Validation and acceptance

The programme is behaviour-preserving, so the existing suite is the primary
acceptance harness: every commit ends with `make check-fmt`, `make lint`, and
`make test` green with no test weakened or deleted. Enriching REST stub bodies
toward realistic fixtures (PR 1) and adding new tests is permitted; loosening
assertions is not.

Red-green-refactor evidence is required for the genuinely new units:

- PR 2: if any helper with observable logic is added to the transport
  (beyond direct delegation), specify it with a failing test first; otherwise
  the characterization harness (retry counts, error text, transcript replay)
  stands in, and that substitution is recorded here.
- PR 3: the `CursorVariables` trait, `paginate_operation`, and each
  `ResponseData` → domain conversion get rstest cases against fixture JSON
  written before the implementation; run the focused test, observe the expected
  failure, implement, observe the pass, then run the wider gates.

End-to-end observations per PR:

- PR 1: `cargo test --features unstable-rest-resolve --test resolve`
  passes; a manual run against a loopback stub shows the reply POST hitting
  `repos/{owner}/{name}/pulls/{n}/comments/{id}/replies` exactly as on `main`.
- PR 2: `cargo tree -i reqwest` fails to find the package; the ignored
  transcript-replay test (`cargo test --test e2e -- --ignored e2e_pr_42`)
  passes, proving transcript format parity; with no server listening, running
  `cargo run -- pr <url>` with `GITHUB_TOKEN=dummy` and
  `GITHUB_GRAPHQL_URL=http://127.0.0.1:1/graphql` fails with a
  connection-refused `RequestContext` error naming the operation, exactly as on
  `main`.
- PR 3: a deliberate field typo in a `.graphql` document turns into a
  compile error (demonstrated once and reverted); the insta snapshots in
  `tests/cli.rs` are byte-identical.

Quality criteria: all gates above; documentation gates (`make markdownlint`,
`make nixie`) for the ADR, design-doc, and `graphql/README.md` changes;
build-time delta within the stated tolerance for PR 3.

PR 1 extracts the URI-normalization and status-classification invariants into
pure helpers. `proptest` exercises arbitrary trailing-slash counts and every
valid HTTP status code: normalization removes all trailing slashes and is
idempotent, while classification treats only 404 as warn-and-continue, 2xx as
success, and every other status as fatal. The integration tests retain exact
route, header, body, authentication, and total-deadline coverage for the raw
Octocrab request.

The REST reply boundary also emits three bounded observability signals:
`vk.resolve.rest_reply.requests.total` counts attempts,
`vk.resolve.rest_reply.duration.seconds` records elapsed time, and
`vk.resolve.rest_reply.timeouts.total` counts total-deadline expirations. The
counter and histogram use only the fixed `outcome`, `status_class`, and
`failure_category` labels. Their values are respectively `success`,
`not_found`, or `failure`; `1xx`, `2xx`, `3xx`, `4xx`, `5xx`, `other`, or
`none`; and `none`, `http_status`, `timeout`, or `transport`. The timeout
counter is unlabelled; no request identifiers, routes, repository names, or raw
errors enter metric labels.

## Idempotence and recovery

Every step is an ordinary source edit gated by the test suite; re-running any
command is safe. If a plan step fails, inspect `git diff --name-only` and use
targeted `git restore -- path/to/reviewed-file` only for files changed by that
failed step. Preserve unrelated work and completed commits; do not use an
unconditional hard reset. The schema download is re-runnable and
version-controlled once vendored. Nothing in this plan touches user data, CI
configuration, or anything outside the repository; the only files written
outside it are `tee` logs under `/tmp`.

## Artifacts and notes

PR 1 artifact and evidence: Octocrab is configured with the recorded feature
set, `tests/resolve.rs` verifies the raw reply route, headers, body,
authentication, status handling, URI normalization, and total deadline, and the
recorded PR 1 gate run is green. The remaining artifacts are the PR 2
`cargo tree -d` duplicate report and clean-build comparison, followed by the PR
3 sample transcript line, schema/code-generation evidence, and closing test
counts.

## Interfaces and dependencies

Dependencies to add:

```toml
# PR 1. Tilde pin: octocrab has shipped breaking changes in patch
# releases (upstream issue 899); widen only after review.
# jwt-rust-crypto is mandatory under default-features = false.
octocrab = { version = "~0.54", default-features = false, features = [
    "default-client", "jwt-rust-crypto", "rustls", "rustls-ring", "timeout",
] }

# PR 2 (promoted from dev-dependencies; align versions with the lockfile)
hyper = "1"
hyper-util = { version = "0.1", features = ["client", "client-legacy", "http1", "tokio"] }
hyper-rustls = { version = "0.27", features = ["webpki-roots", "http1", "ring"] }
http-body-util = "0.1"

# PR 3
graphql_client = "0.16"
```

Dependency to remove (PR 2): `reqwest`.

In `src/api/client/transport.rs` (PR 2, new, private to `client`):

```rust
/// Owns the pooled hyper client used for GraphQL requests.
pub(super) struct Transport { /* hyper_util legacy client + HTTPS connector */ }

impl Transport {
    pub(super) fn new() -> Result<Self, VkError>;

    /// POST `payload` to `endpoint` with `headers`, honouring `timeout`
    /// across the whole request, returning status and body.
    pub(super) async fn post_json(
        &self,
        endpoint: &Endpoint,
        headers: &http::HeaderMap,
        payload: &serde_json::Value,
        timeout: std::time::Duration,
    ) -> Result<HttpResponse, VkError>;
}
```

In `src/api/client/mod.rs` (PR 3, added; string-based methods retained until
unused, then deprecated per the Interface tolerance):

```rust
/// Execute a codegen'd GraphQL operation using this client.
pub async fn run_operation<Q: graphql_client::GraphQLQuery>(
    &self,
    variables: Q::Variables,
) -> Result<Q::ResponseData, VkError>;
```

In `src/api/pagination.rs` or a sibling module (PR 3):

```rust
/// Implemented by generated Variables types for paginated operations.
pub(crate) trait CursorVariables {
    fn set_cursor(&mut self, cursor: Option<String>);
}
```

Unchanged public surface (re-exported from `src/lib.rs` and `src/api/`):
`GraphQLClient` and its constructors, `Endpoint`, `Token`, `Query`,
`RetryConfig`, `paginate`, `PageInfo`, `CommentConnection`, `ReviewThread`,
`ReviewComment`, `PullRequestReview`, `Issue`, `User`, and `VkError`.

In `src/resolve/rest.rs` (PR 1), `RestClient` keeps its `pub(crate)`
construction and the `post_reply` free function; only the inner client type
changes from `reqwest::Client` to `octocrab::Octocrab`.

## Revision note

2026-07-09: initial draft proposed octocrab as the transport for both GraphQL
and REST.

2026-07-09 (second revision): scope changed on user direction after review of
the first draft. octocrab is now confined to the REST resolve path; the bespoke
GraphQL client is retained for its observability machinery and moves from
reqwest to a direct hyper transport; `graphql_client` codegen adds compile-time
query checking. Delivery is three pull requests. The planned ADR was renamed
from `adr-001-adopt-octocrab.md` to
`adr-001-github-api-client-modernisation.md`. Constraints, tolerances, risks,
decisions, and interfaces were rewritten to match. PR 1 is complete; PRs 2 and
3 remain as authorized future work.

# Adopt octocrab as the GitHub API transport for vk

This ExecPlan (execution plan) is a living document. The sections `Constraints`,
`Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`, `Decision Log`,
and `Outcomes & Retrospective` must be kept up to date as work proceeds.

Status: DRAFT

## Purpose / big picture

`vk` is a command-line tool that shows unresolved GitHub pull request review
comments in the terminal. Today it talks to GitHub through two hand-rolled HTTP
clients built directly on the `reqwest` crate: a GraphQL client
(`src/api/client/`) used by every subcommand, and a REST client
(`src/resolve/rest.rs`, compiled only under the `unstable-rest-resolve`
feature) used to post a reply before resolving a review thread.

This plan replaces the `reqwest` transport underneath both clients with
[octocrab](https://docs.rs/octocrab) (version 0.54.x, the maintained GitHub API
client for Rust), while preserving `vk`'s externally observable behaviour:
command output, error messages, retry semantics, transcript recording,
authentication precedence, and the environment-variable endpoint overrides that
the entire test suite depends on.

After this change, a user sees no behavioural difference: `vk pr`, `vk issue`,
and `vk resolve` work exactly as before, and `make test` passes with the same
suite. What the project gains is a maintained transport layer (authentication
plumbing, base-URI handling, connection management, rate-limit-aware retry
available for future use) and a ready-made typed REST surface for future
features, in exchange for deleting bespoke plumbing. Success is observable as:
`cargo tree -i reqwest` reports the crate is absent from the dependency graph,
`cargo tree -p vk | grep octocrab` shows octocrab present, and all existing
gates (`make check-fmt`, `make lint`, `make test`, `make markdownlint`,
`make nixie`) pass.

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
- Requests must keep sending `User-Agent: vk`,
  `Accept: application/vnd.github+json`, and `Authorization: Bearer <token>`
  (only when a token is present; anonymous access must keep working and keep
  printing the "GitHub token not set, using anonymous API access" warning).
- The transcript feature (`VK_TRANSCRIPT` / constructor parameter) must keep
  writing the same JSON-lines format consumed by
  `tests/e2e/common.rs::load_transcript` and the `tests/fixtures/pr42.json`
  fixture: one object per line with keys `operation`, `status`, `request`, and
  `response` (response body truncated to 500 characters).
- Retry behaviour must be preserved: jittered exponential backoff via
  `backon` with the transient-error classification in `src/api/retry.rs` (retry
  on request errors, empty responses, HTTP 5xx, HTTP 429, and HTML-looking
  bodies).
- TLS must remain rustls-based; do not introduce a native-tls or OpenSSL
  dependency.
- Dependency versions must use caret requirements unless a tilde pin is
  documented with a reason (see Decision Log for the octocrab pin).
- The error type `VkError` and its variants remain the error surface of
  `src/api/`; octocrab's `snafu`-based errors must be mapped at the transport
  boundary and never leak into public signatures.
- All commit gates pass on every commit: `make check-fmt`, `make lint`,
  `make test`, and for documentation changes `make markdownlint` and
  `make nixie`.
- No single source file may exceed 400 lines; module-level `//!` comments
  and en-GB-oxendict spelling are required in all new code.

## Tolerances (exception triggers)

- Scope: if the migration requires editing more than 25 source files or a
  net change beyond roughly 1,500 lines, stop and escalate.
- Interface: if a public API of the `vk` library crate (anything re-exported
  from `src/lib.rs`, such as `GraphQLClient`, `Endpoint`, `Token`, `Query`,
  `RetryConfig`, `CommentConnection`, `PageInfo`) must change signature, stop
  and escalate. Internal (private or `pub(crate)`) signatures may change freely.
- Dependencies: adding `octocrab` (and accepting its transitive hyper/tower
  stack) is pre-approved by this plan. Any further new direct dependency
  requires escalation.
- Behaviour: if preserving an asserted behaviour (error text, transcript
  format, header set) proves impossible on top of octocrab, stop and present
  options; do not weaken tests to fit.
- Iterations: if a gate still fails after three fix attempts on the same
  failure, stop and escalate.
- Prototype outcome: if Milestone 1 (the spike) shows octocrab cannot
  expose raw response status and body for the transcript and error-snippet
  machinery, stop and escalate with alternatives (see Risks).

## Risks

- Risk: octocrab's default builder does not accept custom tower layers, so
  the transcript recorder cannot be a middleware; the plan instead relies on
  octocrab's raw `_post` method returning an unprocessed `http::Response` from
  which status and body can be read. Severity: high. Likelihood: low.
  Mitigation: Milestone 1 is a spike proving the raw-response path against the
  existing unit tests before any consumer changes. Fallback: build the client
  via `OctocrabBuilder::new_empty().with_service(...)` with a custom recording
  layer, or escalate.
- Risk: octocrab has shipped breaking changes in patch releases before
  (issue 899, the 0.49.8 GraphQL return-type change). Severity: medium.
  Likelihood: medium. Mitigation: pin with a tilde requirement (`~0.54`) and
  record the reason in `Cargo.toml` comment and the ADR (see Decision Log).
- Risk: unit tests construct clients with endpoints that have no path
  (`http://127.0.0.1:PORT`), while e2e tests use
  `http://127.0.0.1:PORT/graphql`; octocrab joins a relative route onto a base
  URI, so the endpoint-to-(base URI, route) split must handle both. Severity:
  medium. Likelihood: high (it will definitely arise). Mitigation: a dedicated
  pure function with rstest coverage written test-first (Milestone 2, Stage B).
- Risk: double-retry — octocrab's default retry layer (`Simple(3)`) would
  stack with `backon`, tripling observed attempts and breaking the retry unit
  tests that count requests. Severity: medium. Likelihood: high. Mitigation:
  configure `add_retry_config(RetryConfig::None)` on the octocrab builder; keep
  `backon` as the single retry mechanism.
- Risk: octocrab's REST models may not deserialize the minimal inline JSON
  bodies used by `tests/resolve.rs`, breaking the REST reply path if it moves
  to octocrab's typed `reply_to_comment`. Severity: low. Likelihood: medium.
  Mitigation: use the raw `_post` route for the reply as well, preserving the
  exact path and status-code semantics (404 non-fatal, 403/500 fatal); typed
  handlers can be adopted later.
- Risk: binary size and compile time grow while both reqwest and octocrab
  are present mid-migration. Severity: low. Likelihood: certain but transient.
  Mitigation: the final milestone removes reqwest; the overlap is bounded to
  the life of this branch.

## Progress

- [x] (2026-07-09 12:20Z) Explored codebase (client layer, consumers, test
  infrastructure, docs) and researched octocrab 0.54; findings folded into this
  plan.
- [x] (2026-07-09 12:40Z) ExecPlan drafted and linked from
  `docs/contents.md`.
- [ ] Stage A: author ADR `docs/adr-001-adopt-octocrab.md`; add octocrab
  dependency; record pin rationale.
- [ ] Milestone 1 (prototype): swap `GraphQLClient` transport to octocrab
  behind the existing facade; `src/api/client/tests.rs` passes unchanged.
- [ ] Milestone 2: endpoint-splitting function (test-first), transcript,
  redaction, retry, and timeout parity; full `make test` green.
- [ ] Milestone 3: REST resolve path on octocrab
  (`--features unstable-rest-resolve`); `tests/resolve.rs` green.
- [ ] Milestone 4: remove reqwest; update `docs/vk-design.md`,
  `docs/repository-layout.md`, `docs/developers-guide.md` as needed; final
  gates and retrospective.

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
  configured anywhere. Impact: the migration only needs to preserve the env-var
  overrides, not proxy semantics; the guide should be corrected during
  Milestone 4.
- Observation: the transcript writes the raw, unredacted request payload;
  redaction (`redact_sensitive`) applies only to error-context snippets.
  Evidence: `src/api/client/transcript.rs` versus
  `src/api/client/mod.rs:126-134`. Impact: parity is the goal; do not silently
  change redaction behaviour during the migration. Flagged for a possible
  follow-up outside this plan.

## Decision log

- Decision: keep the `GraphQLClient` facade and `VkError` taxonomy; replace
  only the transport internals with octocrab. Rationale: consumers
  (`src/commands.rs`, `src/review_threads.rs`, `src/reviews.rs`,
  `src/issues.rs`, `src/branch_pr/`, `src/resolve/`) and roughly twenty network
  tests couple to the facade, the error text, and the env-var overrides.
  Swapping internals preserves all of that and bounds the blast radius;
  rewriting consumers against octocrab's typed models would triple the diff for
  no behavioural gain. Date/Author: 2026-07-09, planning session.
- Decision: use octocrab's raw `_post` (returning `http::Response`) for
  GraphQL and for the REST reply, not the convenience `graphql()` method or
  typed REST handlers. Rationale: `graphql()` swallows the raw body and
  discards partial-success data, which would break transcript recording,
  body-snippet error context, and the HTML-body transient-retry heuristic.
  `_post` inherits auth and base-URI middleware while leaving response
  processing to `vk`. Date/Author: 2026-07-09, planning session.
- Decision: keep `backon` retry and disable octocrab's retry layer
  (`RetryConfig::None` on the builder). Rationale: a single retry authority
  preserves the tested attempt counts and backoff behaviour. octocrab's
  rate-limit-aware retry cannot see GraphQL rate-limit errors (they arrive as
  HTTP 200 with an `errors` payload), so `backon` at the operation level
  remains necessary anyway. Date/Author: 2026-07-09, planning session.
- Decision: pin octocrab with a tilde requirement (`~0.54`).
  Rationale: octocrab has a documented history of breaking changes in patch
  releases (upstream issue 899). AGENTS.md permits tilde pins where the reason
  is documented; this is that documentation, and the ADR repeats it.
  Date/Author: 2026-07-09, planning session.
- Decision: record the adoption in a new ADR,
  `docs/adr-001-adopt-octocrab.md`. Rationale: no ADRs exist; the
  bespoke-client choice was never recorded. AGENTS.md requires substantive
  decisions to be captured as ADRs following
  `docs/documentation-style-guide.md` (Status, Date, Context and Problem
  Statement, Options Considered, Decision Outcome, Migration Plan, Known
  Risks). Date/Author: 2026-07-09, planning session.

## Outcomes & retrospective

To be completed as milestones land and at the end of the work.

## Context and orientation

The repository is a single Rust crate (`vk`, edition 2024, MSRV 1.89) with
sources under `src/` and integration tests under `tests/`. The `Makefile` is
the canonical command runner. Read `AGENTS.md` before contributing.

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
wire shape; these structs are unchanged by this plan.

The REST path: `src/resolve/rest.rs` (only compiled with
`--features unstable-rest-resolve`) builds a second `reqwest::Client`
(`github_client`) and a `RestClient` whose base URL comes from the
`GITHUB_API_URL` env var (default `https://api.github.com`). Its one operation,
`post_reply`, POSTs to
`repos/{owner}/{name}/pulls/{pull_number}/comments/{comment_id}/replies`,
treating 404 as non-fatal and other non-2xx as fatal, with no retry.

Authentication: `src/auth.rs::resolve_github_token` implements the precedence
chain; no `gh` CLI integration exists. The token is a plain string handed to
the client constructors.

Tests that constrain this work:

- `src/api/client/tests.rs` spins up loopback hyper servers and constructs
  clients via `with_endpoint`/`with_endpoint_retry` using endpoints without a
  path; it counts retry attempts and asserts error-text fragments.
- `src/test_utils/test_http.rs` and `src/branch_pr/tests.rs` follow the same
  pattern.
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

Key octocrab facts (version 0.54.0, MSRV 1.85, rustls + ring by default):

- It builds on hyper 1.x and tower, not reqwest. `OctocrabBuilder`
  provides `personal_token`, `base_uri` (a tower layer rewriting relative
  routes, applying to REST and GraphQL alike), `add_header`,
  `add_retry_config`, and connect/read/write timeouts.
- `Octocrab::_post(route, body)` returns the raw `http::Response`, from
  which status and body bytes can be read — this is the hook that preserves
  `vk`'s transcript, snippets, and retry classification.
- The convenience `graphql()` method deserializes internally and discards
  the raw body and partial-success data; it is not used by this plan.
- Resolving a review thread has no REST endpoint; the `resolveReviewThread`
  GraphQL mutation remains the only way, so GraphQL stays central.

## Plan of work

The work is four milestones, each ending in a commit (or small commit series)
with all gates green. The public facade means consumers are untouched until
Milestone 3, and most tests act as a characterization harness throughout: their
continuing to pass is the acceptance evidence.

Stage A (part of the first commit): author `docs/adr-001-adopt-octocrab.md`
following the ADR template in `docs/documentation-style-guide.md` (Status:
Accepted; Context: two bespoke reqwest clients, undocumented prior decision;
Options Considered: keep bespoke, octocrab, graphql_client + reqwest codegen;
Decision Outcome: octocrab as transport with retained facade; Migration Plan:
reference this ExecPlan; Known Risks: semver history, hence tilde pin).
Reference the ADR from `docs/vk-design.md` and list it in `docs/contents.md`.
Add the dependency to `Cargo.toml`:

    octocrab = { version = "~0.54", default-features = false, features = ["rustls", "rustls-ring", "timeout"] }

(Feature list to be verified during the spike: the goal is rustls with the ring
provider, timeouts available, octocrab's retry and tracing layers not required;
adjust to the minimal set that compiles, and record the final set here.) Run
`cargo tree -d` to check for duplicate major versions of shared transitive
dependencies and note findings in `Surprises & discoveries`.

Milestone 1 (prototyping, go/no-go): inside `src/api/client/`, add a private
transport module (`src/api/client/transport.rs`) that owns an
`octocrab::Octocrab` instance plus the route to post to, and give it one async
method mirroring today's `execute_single_request` contract: take a JSON
payload, return `HttpResponse { status, body }` or `VkError`. Construct it from
the existing `Endpoint`, `Token`, and `RetryConfig` values:

- Split the `Endpoint` URL into a base URI and a route path (for
  `http://127.0.0.1:9999` the route is `/`; for
  `https://api.github.com/graphql` the base is `https://api.github.com` and the
  route `/graphql`). This is the pure function `split_endpoint` described under
  Interfaces below, written test-first.
- Builder: `personal_token` only when the token is non-empty; `add_header`
  for `User-Agent: vk` and `Accept: application/vnd.github+json` (verify during
  the spike whether octocrab's defaults duplicate or fight these; the wire
  assertions in `tests/auth.rs` are the arbiter);
  `add_retry_config(octocrab::service::middleware::retry::RetryConfig::None)`;
  read/connect timeouts from `vk`'s `RetryConfig::request_timeout`.
- Method body: `_post(route, Some(&payload))`, read status, collect body
  bytes to a `String`, map transport errors into `VkError::RequestContext` with
  the same context text as today.

Switch `execute_single_request` to delegate to the transport while leaving
everything else (transcript call, status check, snippets, retry loop)
untouched. The `reqwest::Client` field and `headers: HeaderMap` field of
`GraphQLClient` are removed or bypassed. Acceptance: `cargo test api::client`
(module tests) and then `make test` pass without any test edits. If raw
status/body access or header parity cannot be achieved, stop: this is the
go/no-go gate.

Milestone 2 (parity hardening): promote the spike to final quality. Red-green
applies to the new unit: add rstest cases for `split_endpoint` covering
path-less endpoints, `/graphql` paths, deeper paths, and trailing slashes,
written before the implementation and observed failing (the function is new, so
the red stage is a compile-fail or a failing assertion against a stub). Confirm
timeout behaviour (a hanging stub server test already exists in the retry
tests; verify it still passes), transcript output (run the ignored `e2e_pr_42`
transcript test locally: `cargo test --test e2e -- --ignored e2e_pr_42` — it
replays `tests/fixtures/pr42.json`), and anonymous access (`tests/auth.rs`).
Delete `src/api/client/helpers.rs::build_headers` and its two unit tests only
if header construction has genuinely moved into the octocrab builder; otherwise
keep it as the single source of header values fed to `add_header`. Update module
`//!` comments to describe the octocrab transport. All gates green; commit.

Milestone 3 (REST resolve path): replace `src/resolve/rest.rs::github_client`
and the `RestClient` internals with a second octocrab instance whose base URI
comes from the existing resolution (parameter, then `GITHUB_API_URL`, then
default). Keep `post_reply`'s signature and semantics: build the same relative
route string, call `_post`, keep 404-as-warning and other-non-2xx-as-fatal
mapping into `VkError`. Do not adopt the typed `pulls().reply_to_comment`
handler in this plan (see Decision Log). Acceptance:
`cargo test --features unstable-rest-resolve` passes, including
`tests/resolve.rs`; `make lint` (which uses `--all-features`) passes. Commit.

Milestone 4 (removal and documentation): delete the `reqwest` dependency from
`Cargo.toml` and any residual `use reqwest` imports (after Milestones 1–3 the
only candidates are error-source downcasts; replace with hyper/http
equivalents). Verify with `cargo tree -i reqwest` (expected: error, package not
found). Update documentation:

- `docs/vk-design.md`: rewrite the networking section to describe the
  octocrab transport, retained facade, retry split (backon outside, octocrab
  retry disabled), and endpoint splitting; reference the ADR.
- `docs/vk-end-to-end-testing-guide.md`: correct the third-wheel
  description (loopback stub via env-var override, not a MITM proxy).
- `docs/repository-layout.md` and `docs/developers-guide.md`: only if
  module boundaries moved (they should not).
- `docs/contents.md`: ensure the ADR and this plan are listed.

Run all gates including `make markdownlint` and `make nixie`. Complete
`Outcomes & retrospective`. Set Status to COMPLETE. Commit.

## Concrete steps

All commands run from the repository root
(`/home/leynos/Projects/vk.worktrees/adopt-octocrab`). Long outputs go through
`tee` to a log file for review, for example:

    make test 2>&1 | tee "/tmp/test-vk-adopt-octocrab.out"

Per-milestone sequence (repeat for each milestone):

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

    cargo test api::client 2>&1 | tee /tmp/test-vk-adopt-octocrab.out
    cargo test --features unstable-rest-resolve --test resolve 2>&1 | tee /tmp/test-vk-adopt-octocrab.out
    cargo test --test e2e -- --ignored e2e_pr_42 2>&1 | tee /tmp/test-vk-adopt-octocrab.out
    cargo tree -i reqwest        # Milestone 4: expect "package … not found"
    cargo tree -d                # check duplicate transitive versions

Expected shape of a passing test run (counts will drift as tests are added):

    test result: ok. 0 failed; finished in …

## Validation and acceptance

The migration is behaviour-preserving, so the existing suite is the primary
acceptance harness: every milestone ends with `make check-fmt`, `make lint`, and
`make test` green with no test weakened or deleted (except the two
`build_headers` unit tests, which may move with the code they test — see
Milestone 2).

Red-green-refactor evidence is required for the one genuinely new unit:

- Red: add `split_endpoint` rstest cases in the transport module first;
  run `cargo test api::client::transport` and observe failure (initially a
  compile failure for the missing function, then failing assertions against a
  todo stub).
- Green: implement `split_endpoint` minimally; the focused test passes.
- Refactor: tidy, then run `make lint` and `make test`.

End-to-end observation: with no token and pointing at a loopback stub, the
binary behaves identically before and after, for example:

    GITHUB_GRAPHQL_URL=http://127.0.0.1:1/graphql GITHUB_TOKEN=dummy \
      cargo run -- pr https://github.com/leynos/vk/pull/1

fails with a connection-refused `RequestContext` error mentioning the operation
name, exactly as on `main`.

Quality criteria: all gates above, plus `cargo tree -i reqwest` failing to find
the package at the end of Milestone 4, and documentation gates for the ADR and
design-doc updates.

## Idempotence and recovery

Every step is an ordinary source edit gated by the test suite; re-running any
command is safe. Each milestone is an independent commit, so a failed milestone
is abandoned with `git restore` / `git reset --hard HEAD` without affecting
completed work. Nothing in this plan touches user data, CI configuration, or
anything outside the repository; the only files written outside it are `tee`
logs under `/tmp`.

## Artifacts and notes

Record here, as milestones complete: the final octocrab feature set, the
`cargo tree -d` duplicate report, a sample transcript line proving format
parity, and the closing test counts.

## Interfaces and dependencies

Dependency to add (Stage A; final feature list recorded here after the spike):

    # Tilde pin: octocrab has shipped breaking changes in patch releases
    # (upstream issue 899); widen only after review.
    octocrab = { version = "~0.54", default-features = false, features = ["rustls", "rustls-ring", "timeout"] }

Dependency to remove (Milestone 4): `reqwest`.

In `src/api/client/transport.rs` (new, private to `client`):

    /// Owns the octocrab instance and the GraphQL route derived from the
    /// configured endpoint.
    pub(super) struct Transport {
        octocrab: octocrab::Octocrab,
        route: String,
    }

    impl Transport {
        pub(super) fn new(
            token: &Token,
            endpoint: &Endpoint,
            retry: &RetryConfig,
        ) -> Result<Self, VkError>;

        /// POST `payload` to the GraphQL route, returning status and body.
        pub(super) async fn post_json(
            &self,
            payload: &serde_json::Value,
        ) -> Result<HttpResponse, VkError>;
    }

    /// Split a full endpoint URL into (base URI, route path).
    ///
    /// `https://api.github.com/graphql` -> ("https://api.github.com", "/graphql")
    /// `http://127.0.0.1:9999`          -> ("http://127.0.0.1:9999", "/")
    pub(super) fn split_endpoint(endpoint: &Endpoint) -> Result<(String, String), VkError>;

Unchanged public surface (re-exported from `src/lib.rs` and `src/api/`):
`GraphQLClient` and its constructors/methods, `Endpoint`, `Token`, `Query`,
`RetryConfig`, `paginate`, `PageInfo`, `CommentConnection`, `ReviewThread`,
`ReviewComment`, `PullRequestReview`, `Issue`, `User`, and `VkError`.

In `src/resolve/rest.rs`, `RestClient` keeps its `pub(crate)` construction and
`post_reply` free function; only the inner client type changes from
`reqwest::Client` to `octocrab::Octocrab`.

## Revision note

2026-07-09: initial draft, based on codebase reconnaissance (client layer,
consumers, test infrastructure, documentation conventions) and octocrab 0.54
research. No implementation has begun; awaiting approval.

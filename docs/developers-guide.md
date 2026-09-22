# Developer's guide

This guide is for maintainers and contributors changing `vk`. It records the
current development workflow and links to the design documents that explain the
system in more depth.

## Normative references

Read these documents before changing behaviour or architecture:

- [AGENTS](../AGENTS.md): repository instructions, Rust rules, and quality
  gates.
- [Documentation contents](contents.md): index of long-lived project
  documentation.
- [GitHub API client modernization ADR](adr-001-github-api-client-modernisation.md):
  accepted REST and GraphQL client decisions.
- [Octocrab adoption ExecPlan](execplans/adopt-octocrab.md): implementation
  plan for the REST client migration.
- [Repository layout](repository-layout.md): path responsibilities and source
  tree conventions.
- [VK design](vk-design.md): application architecture and behaviour rationale.
- [Documentation style guide](documentation-style-guide.md): Markdown,
  spelling, and document-structure rules.
- [End-to-end testing guide](vk-end-to-end-testing-guide.md): E2E test
  strategy, fixture expectations, and transcript handling.

## Build and validation workflow

Use the Makefile targets as the source of truth for local validation:

```bash
make fmt
make check-fmt
make markdownlint
make nixie
make lint
make test
```

Run commands sequentially. The repository relies on shared Cargo caching, and
sequential execution keeps cache use predictable.

Use `make fmt` after documentation changes. It formats Rust and Markdown
sources. Review Markdown formatter diffs before committing so footnotes, links,
tables, and code fences still carry the intended meaning.

## Test expectations

Add or update tests whenever behaviour changes. Use the smallest test layer
that exercises the behaviour:

- Module tests for private helpers and narrow parsing behaviour.
- Integration tests under `tests/` for command-line, configuration, and public
  workflow behaviour.
- End-to-end tests under `tests/e2e/` for externally observable workflows.
- Snapshot tests where output format consistency matters.

Environment-mutating tests must use the shared guards and sandbox helpers under
`src/test_utils.rs` or `tests/support/`. Direct unsynchronised environment
mutation can make tests order-dependent.

## Configuration development

CLI and configuration structures live in [src/cli_args.rs](../src/cli_args.rs).
Global configuration loading lives in
[src/config_loader.rs](../src/config_loader.rs), which preserves configuration
discovery inputs before subcommand parsing.

When adding a user-facing option:

1. Add the field to the relevant argument structure.
2. Define merge and precedence behaviour through `ortho_config`.
3. Add tests for CLI, environment, and file configuration precedence.
4. Update [User's guide](users-guide.md) and the
   [Ortho Config users' guide](ortho-config-users-guide.md) when the option
   affects user configuration.

## API and command boundaries

GitHub API behaviour belongs under [src/api/](../src/api/). Command
orchestration belongs under [src/commands/](../src/commands/), while terminal
rendering belongs under [src/printer/](../src/printer/). Keep changes within
the module that owns the behaviour unless a shared abstraction has a clear
reuse policy.

Before extracting a helper, port, or abstraction, check whether one already
exists and document the new ownership boundary in the relevant design or
developer document.

### REST resolve client

REST review-comment replies use octocrab through its raw `_post` route. The
`http` and `octocrab` entries in `Cargo.toml` are direct dependencies: `http`
supplies the header types, while octocrab owns authentication, base-URI
handling, and request execution. The octocrab `retry` feature remains excluded
so the reply path stays retry-free.

The connection timeout maps to octocrab's connect timeout. The request timeout
configures octocrab's read and write timeouts and also wraps the complete
`_post` operation, preserving a total deadline across connection, write, and
response-read work.

Status interpretation remains in `vk`: the resolve client warns and continues
for HTTP 404, accepts other successful statuses, and maps every other non-2xx
status to `VkError::RequestContext` with the route and status.

The REST reply boundary emits bounded metrics through the `metrics` crate. The
`vk.resolve.rest_reply.requests.total` counter and
`vk.resolve.rest_reply.duration.seconds` histogram use the fixed `outcome`,
`status_class`, and `failure_category` labels. `outcome` is one of `success`,
`not_found`, or `failure`; `status_class` is one of `1xx`, `2xx`, `3xx`, `4xx`,
`5xx`, `other`, or `none`; and `failure_category` is one of `none`,
`http_status`, `timeout`, or `transport`. The
`vk.resolve.rest_reply.timeouts.total` counter has no labels. These signals
measure attempts, elapsed time, and total-deadline expirations without adding
repository, comment, route, or raw-error values to metric cardinality. The
module records metrics through the configured recorder and does not install a
global recorder itself.

### GraphQL transport

`src/api/client/transport.rs` owns the pooled Hyper/rustls client used by the
bespoke GraphQL client. It supports HTTPS for production endpoints and plain
HTTP only for loopback stub servers used by tests. `GraphQLClient` owns request
transcripts, non-2xx classification, and the `backon` retry loop; the transport
only executes one HTTP attempt and maps failures to `VkError::RequestContext`.

Each attempt has one total deadline covering request submission and response
collection. Cancelling that deadline drops the in-flight request or body-read
future. Response collection is capped at one MiB before bytes are accumulated,
so chunked and content-length responses cannot make the client retain an
unbounded body.

The transport's private context and collection helpers are only composed by one
transport attempt. Do not call them from `GraphQLClient`: it retains ownership
of retry, transcript, and HTTP-status policy.

The private GraphQL metrics module records one counter and duration histogram
per attempt. Its fixed labels are `outcome` (`success` or `failure`),
`status_class` (`1xx`, `2xx`, `3xx`, `4xx`, `5xx`, `other`, or `none`), and
`failure_category` (`none`, `http_status`, `timeout`, `transport`, `body_read`,
or `body_limit`). Never add tokens, payloads, endpoint URLs, operation names,
repository names, identifiers, response bodies, or raw errors as labels. The
transport's tracing span uses the same bounded classifications and no sensitive
fields.

## Coverage: what each lane owns

Main owns every persistent coverage output. `coverage-main.yml` answers a push
to `main`, generates ratcheted coverage and uploads it to CodeScene.
`coverage.yml` answers a pull request, generates coverage for its own ratchet
check, and contacts CodeScene not at all. `tests/coverage_shape_contract.rs`
holds all of it, deriving what it asserts from each workflow's own triggers
rather than from a list of file names, so adding a workflow asks the question
again instead of slipping past a contract keyed on names.

### Why a pull request never reaches CodeScene

Two reasons, and neither is a preference. CodeScene accepts
`cs-coverage upload` only for branches it analyses, so an upload from a
pull-request head is refused outright. And a pull-request lane that contacts
CodeScene puts a third-party network call, and the token that authenticates it,
on the fork-facing side of the repository.

What replaces the changed-line gate is the ratchet.
`every_pull_request_workflow_ratchets_its_own_coverage` requires
`with-ratchet: 'true'` on every pull-request generator, because a lane
generating coverage without it measures nothing it can fail on. The baseline it
compares against is the one `coverage-main.yml` writes: caches saved on `main`
are readable by every pull-request run.

It judges each lane separately rather than pooling every generator into one
list, since pooling lets a second lane's ratcheting generator stand in for a
lane whose own does not ratchet. A coverage lane is derived as a workflow that
generates coverage, not named: requiring every pull-request workflow to
generate coverage would refuse `dependabot-automerge.yml`, which answers
`pull_request_target` and rightly generates none.

### The publisher is derived, not named

`only_the_publisher_uploads_coverage` asks three things of a workflow before it
may upload: that it answers a push, that it serves no pull request, and that
its push trigger is filtered to `main`.

The second condition is what makes the rule applicable. A repository whose
single workflow declares both triggers would otherwise be required to upload
and forbidden from uploading at the same time, and the contract would have no
consistent reading.

The third is what makes it mean anything. `release.yml` answers a push, of
tags, and serves no pull request, so without the branch filter it qualified as
the publisher and could have carried a CodeScene upload with no contract
objecting. CodeScene accepts an upload only for a branch it analyses, so that
upload would have failed at run time instead of being refused here.

`the_publisher_uploads_rather_than_checks` requires `mode: upload` explicitly
rather than leaving the action's default in force. The default is `upload`
today, so this changes no behaviour; it makes which mode is running readable in
the file, and assertable.

### The token belongs to the upload step, and to nothing else

A secret declared at job level is exported into the environment of every step
the job runs, this repository's own build among them, so a compromised build
dependency can read it. At workflow level it reaches every step of every job,
which is wider still. Declared on the upload step, the blast radius is that one
step.

`the_codescene_token_reaches_the_upload_step_and_nothing_else` makes three
claims rather than one: the step that uploads has the token, no other step has
it, and no wider scope declares it. The first claim is not redundant. The
upload step is guarded on the token being non-empty, so moving the token to the
coverage-generation step would leave a contract asking only "some step has it,
no job has it" perfectly green while the guard went false and the publish
silently stopped happening.

### Reading the workflows

The contracts reach the filesystem through a `cap_std::fs_utf8::Dir` capability
opened once, with `camino` paths, rather than through `std::fs` and ambient
authority. That is this repository's rule for filesystem access generally, and
here it also means a contract cannot read outside the directory it reasons
about.

Parsing covers all three scopes GitHub exports an environment from, workflow,
job and step, because a reader seeing only two would call a workflow clean
while its widest scope held the secret.

### Markdown is linted through the pinned action alone

`markdown_is_linted_only_through_the_pinned_action` checks both directions: the
action is pinned to the commit `21c1be1b93ad9ed58fa840aacc3f279cde2a72ff`, and
no step invokes the linter from a shell. The commit matters because the
previous pin, `4580e161`, is the *annotated tag object* for v24.2.0. A tag
object's SHA is immutable, so the old pin was not unsafe, but it is not a
commit, and the estate's rule asks for the commit the tag points at. The shell
direction matters because a `run:` invocation takes whatever linter version the
runner image carries, which is not a pin at all.

## Documentation maintenance

Update documentation in the same branch as the behaviour it describes:

- User-facing behaviour belongs in [User's guide](users-guide.md).
- Maintainer workflow or implementation conventions belong in this guide.
- Architecture and rationale belong in [VK design](vk-design.md).
- Path ownership belongs in [Repository layout](repository-layout.md).
- Significant accepted decisions should become Architecture Decision Records
  (ADRs) following the
  [documentation style guide](documentation-style-guide.md).

Keep [Documentation contents](contents.md) synchronized whenever documents are
added, renamed, or removed.

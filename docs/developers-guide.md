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
`not_found`, or `failure`; `status_class` is one of `1xx`, `2xx`, `3xx`,
`4xx`, `5xx`, `other`, or `none`; and `failure_category` is one of `none`,
`http_status`, `timeout`, or `transport`. The `vk.resolve.rest_reply.timeouts.total`
counter has no labels.
These signals measure attempts, elapsed time, and total-deadline expirations
without adding repository, comment, route, or raw-error values to metric
cardinality. The module records metrics through the configured recorder and
does not install a global recorder itself.

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

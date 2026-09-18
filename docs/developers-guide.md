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

## CI lanes: where each one runs and what it may bill

This repository's pull-request, push and tag lanes run on Ubicloud
(`ubicloud-standard-2`) and are billed per minute. Its manual and automation
lanes stay on GitHub-hosted runners, where minutes are free for a public
repository. `tests/workflow_placement_contract.rs` holds all of it, deriving
what it asserts from each workflow's own triggers rather than from a list of
job names, so adding a lane asks the placement question again instead of
slipping past a contract keyed on names.

### The fork fallback

A pull request from a fork cannot obtain a paid runner, so a pull-request lane
naming one outright would leave every fork contributor's pull request with no
runner at all. Both lanes in `coverage.yml` therefore choose:

```yaml
    runs-on: >-
      ${{ github.event.pull_request.head.repo.fork
      && 'ubuntu-latest' || 'ubicloud-standard-2' }}
```

The continuation sits at the same indent as the first line of the scalar. A
continuation indented one level deeper keeps its line break, and GitHub
evaluates the broken value regardless, so a green run is not evidence that the
expression is well formed. `no_runner_selection_carries_an_embedded_line_break`
reads every `runs-on` from the parsed document and refuses one containing a
break, which is the only way to tell the two apart.

`every_pull_request_lane_falls_back_for_a_fork` checks three separate things:
that the lane chooses rather than naming a label outright, that it chooses on
the fork field rather than on some other field that happens to read similarly,
and that the two arms are exactly the hosted label and the paid one. Reading
the arms rather than matching the whole expression against a pattern means a
lane that is correctly placed but merely wrapped differently still passes.

### Ceilings

Every paid lane declares `timeout-minutes`. Without one a job inherits GitHub's
six-hour default, which costs nothing on a hosted runner and is the expensive
failure mode on a per-minute one.

The values are sized from measured work rather than chosen. Taken on 2026-09-17
from the last three green pull-request runs and the last green push run:

| Job                     | Queue (s) | Run (s)       |
| ----------------------- | --------- | ------------- |
| `build-test`            | 2         | 237, 302, 420 |
| `unstable-rest-resolve` | 2         | 238, 291, 440 |
| `coverage-upload`       | 3         | 184           |

_Table 1: Measured queue and run times per job before the move._

Two things follow. There is no queueing to relieve here, so the case for the
move is consistency and the fork-fallback shape rather than contention. And the
ceilings are sized against those figures doubled, because these lanes move from
four vCPUs to two: a ceiling sized from the hosted figure would cancel an
ordinary run rather than a hung one.
`every_paid_lane_declares_a_bounded_ceiling` holds each lane inside a measured
band, and refuses a paid lane it has no measurement for at all.

### Trunk and tag lanes

`every_trunk_and_tag_lane_runs_on_the_paid_runner` requires every lane in a
push or tag workflow to name `ubicloud-standard-2` outright. Without it the
suite had a hole a reviewer found: reverting `coverage-main.yml` to
`ubuntu-latest` left all six other contracts passing. The ceiling contract
inspects only lanes already on the paid runner, so a lane leaving that set
leaves its scope; and the registry stays balanced because the pull-request
lanes keep the label in use. A contract suite can be individually sound and
still leave a change undetected.

The predicate asks two things before a workflow counts: that it answers a push,
**and** that it serves no pull request. A workflow declaring both owes the fork
fallback, so requiring it to name the paid label outright would contradict the
fallback rule. No workflow here declares both today, which is why the predicate
has to say so now rather than when one is added.

### Properties, and what is not one

Most of these contracts are claims about five checked-in files. Generating
arbitrary workflows would not make them stronger, because their subject is this
repository's configuration rather than the space of possible configurations.

Two readings are different, and `parser_properties` states them as properties.
`arms_of` parses an expression a maintainer writes by hand, so it is driven
over arbitrary arm counts, orders and surrounding whitespace, and over a
malformed expression whose final quote is missing: a reader inventing an arm
from the dangling run would let a mistyped fallback satisfy the fork-fallback
contract. `events_of` reads a trigger block whose YAML shape varies between
mapping, sequence and scalar, with the `on` key quoted or bare. A reader
understanding one shape would report the other workflows as answering nothing,
and every contract keyed on a trigger would then pass over an empty set while
appearing to assert something.

### The actionlint registry

actionlint rejects a `runs-on` label it does not know, so `ubicloud-standard-2`
is registered in `.github/actionlint.yaml`. The contract holds the registry and
the labels in use **equal in both directions**.

The second direction is the substantive one. A subset assertion catches an
unregistered label, which actionlint would have caught anyway, but it says
nothing about a registration left behind after its lane moved away, and that
stale entry is what lets a second paid provider's label appear in a workflow
without anyone deciding to pay for it. The labels in use are derived from both
arms of a conditional and from every job, minus a frozen set of GitHub-hosted
labels. The set is frozen rather than derived from a prefix, because a prefix
test would let a second provider's labels pass as hosted and so escape the
registry question entirely, which is the defect being guarded against.

Jobs that call a reusable workflow are exempt: they declare no `runs-on`, and
the label is the called workflow's business rather than this repository's.

### What stays hosted

`delayed-pr-comment.yml` answers only `workflow_dispatch`, and
`dependabot-automerge.yml` answers `pull_request_target` and
`workflow_dispatch`. Neither is a pull-request or push lane, so
`api_bound_lanes_stay_on_hosted_runners` requires them to stay where minutes
are free.

### Job names

`no_required_context_interpolates_its_runner` refuses an expression in
whichever of the two a required context is derived from: a job's `name` when it
declares one, and its identifier otherwise. Either one carrying the runner
label changes the context when the label does, and the ruleset then requires a
context that nothing reports.

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

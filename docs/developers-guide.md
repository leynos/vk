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
check, and contacts CodeScene not at all. The decision and its alternatives are
recorded in [ADR 002](adr-002-main-owns-coverage-publication.md).

The contracts holding all of it live in one test binary,
`tests/workflow_contracts.rs`, with its modules under
`tests/workflow_contracts/` and the shared reader under
`tests/support/workflows/`. They derive what they assert from each workflow's
own triggers and calls rather than from a list of file names, so adding a
workflow asks the question again instead of slipping past a contract keyed on
names. One binary rather than one per contract means the reader is compiled
once, and every item in it is used by the binary that compiles it, so no
`dead_code` allowance is needed.

### Why a pull request never reaches CodeScene

Two reasons, and neither is a preference. CodeScene accepts
`cs-coverage upload` only for branches it analyses, so an upload from a
pull-request head is refused outright. And a pull-request lane that contacts
CodeScene puts a third-party network call, and the token that authenticates it,
on the fork-facing side of the repository.

`no_pull_request_lane_contacts_codescene` refuses three routes to CodeScene:
the CodeScene action, a `cs-coverage` command, and any other mention of
CodeScene's host, such as a `curl`. It also refuses the token in any scope,
read under any name, and forwarding it with `secrets: inherit`.

The rule runs over the pull-request *closure*, not over the workflows whose own
triggers name a pull request. A reusable workflow declaring only
`workflow_call` runs on behalf of every lane that calls it, and
`secrets: inherit` hands it every secret the caller holds, so a contract
enumerating workflows by trigger alone would never look inside it. A local call
is recognized by shape: strip a leading `./`, then ask whether the rest is a
path under `.github/workflows/`. A call naming a workflow that does not exist
is an error rather than a dead end.

What replaces the changed-line gate is the ratchet.
`every_pull_request_workflow_ratchets_its_own_coverage` requires
`with-ratchet: 'true'` on every generator in the closure, because a lane
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

`only_the_publisher_uploads_coverage` asks four things of a workflow before it
may reach CodeScene: that it answers a push, that it serves no pull request,
that its push trigger is filtered to exactly `main`, and that it names no tag
filter.

The second condition is what makes the rule applicable. A repository whose
single workflow declares both triggers would otherwise be required to upload
and forbidden from uploading at the same time, and the contract would have no
consistent reading.

The third and fourth are what make it mean anything. The branch set must *equal*
`{main}`: `branches: [main, release]` contains `main` and publishes from
`release`. `release.yml` answers a push of tags and serves no pull request, so
without the filter conditions it would qualify as the publisher and could carry
a CodeScene upload with no contract objecting.

`the_publisher_uploads_rather_than_checks` classifies every CodeScene step in
the publisher by operation. The action must say `mode: upload` explicitly
rather than leave the default in force, and a `cs-coverage` command must run
`upload`; `check`, an unstated mode, or a bare request to the host are all
refused, and a script that uploads cannot hide a check beside it.

### The upload is guarded on its token and on `main`

`the_upload_runs_only_on_main_with_its_token` requires the upload step's `if`
to carry two conjuncts, `env.CS_ACCESS_TOKEN != ''` and
`github.ref == 'refs/heads/main'`, and the action's `access-token` input to read
`${{ env.CS_ACCESS_TOKEN }}`. The ref guard is needed as well as the trigger's
branch filter because the publisher also answers `workflow_dispatch`, which
runs against whichever branch the dispatcher picks.

The condition is split on `&&` outside quoted strings, and an unquoted `||` is
refused outright. A substring test for the ref guard passes
`... && github.ref == 'refs/heads/main' || github.event_name == 'workflow_dispatch'`,
which makes every conjunct optional and uploads a dispatch from any branch.

`the_publisher_queues_rather_than_cancels` refuses `cancel-in-progress` on the
publisher at workflow and job level, and `coverage-main.yml` declares a
concurrency group that queues. A cancelled publisher abandons both its upload
and the ratchet baseline it writes; a queued one publishes after the run ahead
of it. Anything but an absent or literally false value counts as cancelling, an
expression included, since a contract cannot promise what an expression will
decide at run time. Cancelling superseded runs remains right for pull-request
lanes.

### The token belongs to the upload step, and to nothing else

A secret declared at job level is exported into the environment of every step
the job runs, this repository's own build among them, so a compromised build
dependency can read it. At workflow level it reaches every step of every job,
which is wider still. Declared on the upload step, the blast radius is that one
step.

`the_codescene_token_reaches_the_upload_step_and_nothing_else` makes three
claims rather than one: the step that uploads reads the token at
`env.CS_ACCESS_TOKEN` and nowhere else, no other step reads it, and no wider
scope reads or forwards it. The first claim is not redundant. The upload step
is guarded on the token being non-empty, so moving the token to the
coverage-generation step would leave a contract asking only "some step has it,
no job has it" perfectly green while the guard went false and the publish
silently stopped happening.

"Reads" means any text value that mentions the secret, found by walking the
whole node rather than the keys a field was written for: an `env` value under
another name, an input, a script, a condition, or a named `secrets:` entry.
GitHub resolves context properties case-insensitively and accepts dotted and
indexed spellings, so `secrets.cs_access_token` and
`secrets['CS_ACCESS_TOKEN']` count, as does `toJSON(secrets)`, which serializes
every secret at once.

### Reading the workflows

The reader parses with `serde_norway`, the maintained fork of `serde_yaml` that
the estate's other Rust workflow contracts use. It reads YAML 1.2 and refuses a
mapping that repeats a key, where PyYAML keeps the last value in silence; a
workflow declaring `runs-on` twice would otherwise be read with one label
discarded.

Reading is fallible and says so: `load` returns a `WorkflowError` for a
directory it cannot list, a file it cannot read or parse, YAML that is not a
mapping, or a directory holding no workflows at all, since every contract over
an empty set passes. Nothing is discarded, because a contract that silently
inspects four workflows where there are five reports success for the one it
never saw. The extension test is case-insensitive, since GitHub reads `.YML` as
readily as `.yml`.

`load` takes a `cap_std::fs_utf8::Dir` capability rather than a path. The
fixture opens it once with ambient authority, at the boundary, and nothing
below it takes ambient authority again; the reader's own tests hand it a
temporary directory instead.

Parsing covers all three scopes from which GitHub exports an environment:
workflow, job, and step. A reader seeing only two would call a workflow clean
while its widest scope held the secret. Triggers are read as a scalar, a
sequence, or a mapping, under the `on` key and under the boolean `true` it
becomes in a YAML 1.1 tool, because `on: [push, pull_request]` read as a
mapping would be one event with a strange name.

Every rule is also tested against constructed workflows that break it one way
at a time, and the classifier and condition splitter carry `proptest`
properties over generated, workflow-shaped input. Run against the repository
alone, a rule is only ever seen passing, which is also what a rule that checks
nothing does.

### Markdown is linted through the pinned action alone

`markdown_is_linted_only_through_the_pinned_action` checks both directions: the
action is pinned to the commit `21c1be1b93ad9ed58fa840aacc3f279cde2a72ff`, and
no step invokes the linter from a shell. The commit matters because the
previous pin, `4580e161`, is the *annotated tag object* for v24.2.0. A tag
object's SHA is immutable, so the old pin was not unsafe, but it is not a
commit, and the estate's rule asks for the commit the tag points at. The shell
direction matters because a `run:` invocation takes whatever linter version the
runner image carries, which is not a pin at all.
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
expression is well-formed. `no_runner_selection_carries_an_embedded_line_break`
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

### Every form `runs-on` can take

GitHub accepts `runs-on` as a scalar, as a sequence of labels, or as a `group`/
`labels` mapping. The reader modelled only the scalar and treated everything
else as a job that declares no runner, which meant the other two forms were
invisible to every contract here rather than merely unhandled. A lane written
`runs-on: [self-hosted, ubicloud-standard-8]` names a runner, can name a paid
or unregistered one, and was skipped in silence by the placement, ceiling and
registry contracts alike.

All three forms are now modelled, and a shape GitHub does not accept is refused
loudly rather than read as an absent runner. `Delegated` now means one thing
only: the key is not there, because the job calls a reusable workflow.

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
`workflow_dispatch`. Neither is a pull-request nor a push lane, so
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

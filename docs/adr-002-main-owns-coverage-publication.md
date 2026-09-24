# Architectural decision record (ADR) 002: Main owns coverage publication

## Status

Accepted (2026-09-22). The push-to-`main` workflow is the only lane that
contacts CodeScene; pull-request lanes generate coverage for their own ratchet
check and nothing else; the CodeScene token lives on the upload step alone.
Amended 2026-09-25: the token is in no `env` at all (see the addendum below).

## Date

2026-09-22.

## Context and problem statement

`vk` measured coverage in two places. The pull-request lane generated coverage,
ran CodeScene's `cs-coverage check` against the changed lines, and exported the
CodeScene token to its whole job. The push-to-`main` lane uploaded the report.

Two facts made the pull-request half untenable. CodeScene accepts coverage only
for branches it analyses, so anything a pull-request head sends is refused. And
CodeScene switched the coverage gates off on every project, after which
`cs-coverage check` fails on every pull request with "received project-config
isn't valid. Lacks the gates configuration". The coverage command-line
interface is itself pinned and verified by digest; what cannot be pinned is the
call to CodeScene's API, whose answer has changed shape twice. A pull-request
lane that makes that call inherits every such change as a red build.

Separately, the token sat in the pull-request lane's job environment, the
fork-facing side of the repository, where every build step, and so every build
dependency, could read it.

The question is which lane owns coverage publication, what replaces the
changed-line gate on pull requests, and where the token may live.

## Decision drivers

- A pull-request build must not fail on a third-party API it does not
  control.
- A secret must reach the one step that uses it and nothing wider.
- Pull requests still need a coverage gate that can fail.
- The rules must be enforced by tests, since none of them is visible in a
  green run.

## Options considered

### Option A: Keep the check on pull requests

Retain `cs-coverage check` on the pull-request lane. It fails on every pull
request while the gates are off, and would fail again on the next change to
CodeScene's answer. Rejected.

### Option B: Main owns publication, pull requests ratchet

Remove every CodeScene step from pull-request lanes. The push-to-`main`
workflow uploads; pull-request lanes run the coverage ratchet against the
baseline `main` writes, so a pull request that lowers coverage still fails.

| Topic                             | Option A      | Option B |
| --------------------------------- | ------------- | -------- |
| Pull request depends on CodeScene | Yes           | No       |
| Coverage gate on pull requests    | Changed lines | Ratchet  |
| Token on the fork-facing side     | Yes           | No       |

_Table 1: Comparison of the options._

## Decision outcome / proposed direction

Option B. The publisher is derived rather than named: a workflow answering a
push, serving no pull request, filtered to exactly `main` and to no tags. Its
upload step is guarded on the token and on `github.ref == 'refs/heads/main'`,
since the publisher also answers `workflow_dispatch`, and its runs queue rather
than cancel so no upload or baseline is abandoned. The token is declared on the
upload step and read nowhere else.

`tests/workflow_contracts.rs` enforces each rule over the repository's own
workflows, and over the closure of reusable workflows that pull-request lanes
call. The [developers' guide](developers-guide.md#coverage-what-each-lane-owns)
describes each contract and the hole it closes.

## Known risks and limitations

- Coverage for a change reaches CodeScene only after it merges, so CodeScene's
  view of a pull request carries no coverage of its own.
- The ratchet compares totals, not changed lines, so a pull request can add
  untested code while raising coverage elsewhere.
- Merges performed by the automerge workflow's `GITHUB_TOKEN` fire no push
  event, so their coverage is published only by a manual dispatch on `main`.

## Addendum (2026-09-25): the token leaves every `env`

The decision above declared the token on the upload step and read it nowhere
else. That was narrower than a job-level binding, but not narrow enough: the
uploader is a composite action, and a composite action's nested steps inherit
the calling step's `env`, so every step inside the action held the token.

The token is now bound in no `env` at any scope. A
`Check CodeScene token availability` step (id `codescene_token`) runs exactly
`echo "available=${{ secrets.CS_ACCESS_TOKEN != '' }}" >> "$GITHUB_OUTPUT"`,
unconditionally and with no `env`; GitHub evaluates the expression before it
sends the command to the runner, so the shell receives only `true` or `false`.
The upload's guard reads `steps.codescene_token.outputs.available == 'true'`
beside the main-ref guard, and the upload takes
`access-token: ${{ secrets.CS_ACCESS_TOKEN }}` directly. A shell upload is no
longer acceptable, since it could take the token only through `env` or its
script. `tests/workflow_contracts.rs` enforces the new shape, and the
[developers' guide](developers-guide.md) describes it.

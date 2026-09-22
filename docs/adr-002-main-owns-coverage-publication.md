# Architectural decision record (ADR) 002: Main owns coverage publication

## Status

Accepted (2026-09-22). The push-to-`main` workflow is the only lane that
contacts CodeScene; pull-request lanes generate coverage for their own ratchet
check and nothing else; the CodeScene token lives on the upload step alone.

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

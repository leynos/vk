# Debugging Plan: Slow-body timeout regression-test retry

**Generated**: 2026-08-23
**Issue ID**: PR #195 review regression test
**Severity**: low
**Falsification sub-agent**: alchemist
**Planning agent boundary**: This document was prepared by the planning agent.
Falsification must be executed by the named sub-agent, not by the planning
agent.

## Problem Statement

The newly added slow-body timeout test expects one HTTP request, but its server
handler is invoked twice and panics after consuming its one response body. The
test should establish the response status retained on body timeout without
making an unsupported assumption about retry semantics.

## Context Summary

| Aspect              | Details                                              |
| ------------------- | ---------------------------------------------------- |
| First observed      | 2026-08-23 while running the focused regression test |
| Reproduction rate   | One deterministic focused-test failure               |
| Affected components | `RetryConfig`, `backon`, client timeout test fixture |
| Recent changes      | Added a `503` header with a delayed response body    |

### Error Artefacts

```plaintext
serve one response
request failed when running operation SlowBody: client error (SendRequest)
```

### Information Gaps

The exact `backon::with_max_times` interpretation has not yet been verified
against the installed version.

## Hypotheses

### H1: One configured retry follows the timed-out first request

**Claim**: `RetryConfig { attempts: 1 }` permits one retry after the initial
request, and `should_retry` classifies the timeout as transient.

**Plausibility**: High — the fixture's sole response body was consumed before
the second handler invocation.

**Prediction**: The retry builder or its documentation will define the value as
retries rather than total requests, and the existing timeout error will satisfy
the retry classifier.

#### H1 Falsification Plan

| Step | Action                                                                                                                                  | Expected Negative Result                           |
| ---- | --------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------- |
| 1    | Inspect `build_retry_builder` and the installed `backon` API.                                                                           | The configured value caps total attempts at one.   |
| 2    | Run only `run_query_retains_status_when_response_body_times_out` after supplying two delayed bodies in an uncommitted local experiment. | The handler still receives more than two requests. |

**Tooling**: Leta/source inspection and one focused `cargo test` experiment.

**Confidence on falsification**: High; the request count directly distinguishes
retry behaviour from a server-body fixture issue.

## Recommended Execution Order

1. **H1** — it is the only concrete explanation supported by the failure.

## Outcome

H1 was not falsified. `backon` permits one retry after the initial operation for
`with_max_times(1)`, and `RequestContext` errors are retryable. The slow-body
fixture therefore uses `attempts: 0` to keep the status-retention test to one
request. `RetryConfig` documentation now describes the established retry-count
semantics.

## Termination Criteria

- **Root cause identified**: The retry configuration is confirmed to permit a
  second attempt after a transient timeout.
- **Escalation trigger**: The handler is not retried or remains invoked more
  than twice after the minimal fixture experiment.

## Notes for Executing Agent

Run only the supplied minimal experiment. Do not run repository gates and do
not leave tracked-file edits behind. Return one verdict: falsified,
not-falsified, or inconclusive.

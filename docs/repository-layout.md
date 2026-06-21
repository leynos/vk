# Repository layout

This document describes the current repository structure for contributors. It
is the canonical place for path responsibilities and tree-orientation guidance.

## Top-level tree

The following tree is a compact orientation sketch, not an exhaustive file
listing.

```plaintext
.
├── .github/
├── docs/
│   └── execplans/
├── src/
│   ├── api/
│   ├── branch_pr/
│   ├── commands/
│   ├── printer/
│   ├── ref_parser/
│   ├── resolve/
│   └── review_threads/
├── tests/
│   ├── e2e/
│   ├── fixtures/
│   ├── snapshots/
│   ├── support/
│   └── utils/
├── AGENTS.md
├── Cargo.toml
├── Makefile
└── README.md
```

_Figure 1: Compact repository tree for contributor orientation._

## Path responsibilities

- `.github/`: GitHub automation and dependency-management configuration.
- `docs/`: Long-lived user, developer, design, migration, and reference
  documentation.
- `docs/execplans/`: Living implementation plans that record progress,
  decisions, and handoff context.
- `src/`: Rust source code for the `vk` library and command-line application.
- `src/api/`: GitHub API access, pagination, and retry behaviour.
- `src/branch_pr/`: Pull request discovery for the current Git branch.
- `src/commands/`: Command handlers and command-level orchestration.
- `src/printer/`: Terminal output formatting and presentation logic.
- `src/ref_parser/`: GitHub reference parsing and Git integration helpers.
- `src/resolve/`: Review-thread resolution through GraphQL and optional REST
  paths.
- `src/review_threads/`: Review-thread domain modelling and related behaviour.
- `tests/`: Integration, command-line, end-to-end, and regression tests.
- `tests/e2e/`: End-to-end tests that exercise externally observable workflows.
- `tests/fixtures/`: Stable sample data used by tests.
- `tests/snapshots/`: `insta` snapshot output; update deliberately and review
  diffs carefully.
- `tests/support/`: Shared integration-test support code.
- `tests/utils/`: Test utility modules used across the test suite.
- `AGENTS.md`: Repository-specific agent instructions and quality-gate
  expectations.
- `Cargo.toml`: Crate metadata, dependencies, features, and lint configuration.
- `Makefile`: Canonical local commands for formatting, linting, testing, and
  docs checks.
- `README.md`: Public project overview and quick-start documentation.

## Source and test conventions

Source modules are grouped by feature responsibility rather than by technical
layer. Keep related parsing, command, API, and rendering behaviour close to the
module that owns it. Shared code should move only when it has a clear ownership
boundary and reuse policy.

Tests live under `tests/` when they exercise public or integration behaviour.
Module-local tests may live beside implementation files where they validate
private behaviour. Test fixtures and snapshots are long-lived artefacts and
should be updated only when the expected behaviour changes deliberately.

## Generated and build artefacts

Cargo build output belongs in `target/` and is not part of the documented
source tree. Do not commit generated build output. Snapshot files under
`tests/snapshots/` are committed test artefacts, not disposable build output.

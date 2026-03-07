# Execplan: adopt `ortho-config` v0.8.0

## Goal

Upgrade this repository from `ortho-config` v0.6.0 to v0.8.0, preserving the
existing configuration precedence rules for `vk` while aligning the codebase,
tests, and documentation with the v0.8.0 migration requirements.

## Current state

- `Cargo.toml` pins `ortho_config = "0.6.0"` and forwards the optional `toml`,
  `json5`, and `yaml` features from the application crate.
- The application uses derive macros through `#[derive(OrthoConfig)]`, but does
  not declare `ortho_config_macros` directly in `Cargo.toml`.
- The code imports the runtime crate as `ortho_config` everywhere; no crate
  alias is currently in use.
- The repository already uses a recent nightly toolchain in
  `rust-toolchain.toml`, so the Rust 1.88 floor should be satisfied in
  practice. The crate manifest does not currently declare `rust-version`.
- Configuration merge behaviour is covered by integration tests in
  `tests/cli_args_merge.rs` and helpers under `tests/support/`.
- Documentation still advertises `ortho-config` v0.6.0 in `README.md` and in
  the OrthoConfig guides under `docs/`.

## Migration notes mapped to this repo

### Applies directly

- Update the `ortho_config` dependency to `0.8.0`, then refresh `Cargo.lock`.
- Confirm that the active toolchain remains compatible with the new minimum
  Rust version.
- Audit YAML-backed tests and sample configuration for YAML 1.2 compatibility,
  especially any unquoted `yes`, `on`, or `off` literals and any duplicate
  mapping keys.
- Prefer `ortho_config` re-exports if implementation code or tests import
  `figment`, `uncased`, or `xdg` directly during the upgrade.
- Update project documentation that currently references v0.6.0 behaviour.

### Likely no-op, but must be verified

- Crate alias support: no `Cargo.toml` alias for `ortho_config` is currently
  present, so `#[ortho_config(crate = "...")]` should not be needed unless an
  alias appears in another manifest or test-only crate.
- `cli_default_as_absent`: no current use was found in the application code.
  Confirm this remains true across the full tree before implementation.
- Documentation artifact generation: no existing use of `cargo orthohelp`,
  `OrthoConfigDocs`, or `[package.metadata.ortho_config]` was found. Decide
  whether this repository wants to add doc generation now or explicitly defer
  it.

## Implementation plan

### 1. Prepare the dependency and toolchain update

- Change `ortho_config` in `Cargo.toml` from `0.6.0` to `0.8.0`.
- Run a lockfile refresh so the generated dependency graph matches v0.8.0.
- Inspect the resolved `ortho_config` and transitive parser crates in
  `Cargo.lock` to confirm the expected versions landed.
- Decide whether to add `rust-version = "1.88"` to `Cargo.toml`. The nightly
  toolchain likely satisfies the compiler requirement already, but adding the
  manifest floor would make the compatibility contract explicit.

### 2. Reconcile derive and import changes

- Search for every `#[derive(... OrthoConfig ...)]` and any use of
  `SelectedSubcommandMerge` or similar derive-generated APIs.
- If any crate aliasing is introduced or discovered during the dependency
  update, add `#[ortho_config(crate = "...")]` to the affected derive sites.
- Search for direct imports of `figment`, `uncased`, or `xdg`; replace them
  with `ortho_config::figment`, `ortho_config::uncased`, and
  `ortho_config::xdg` where the source is derive-generated or tightly coupled
  to the runtime crate.

### 3. Audit clap default handling

- Search the full tree for `cli_default_as_absent`, `default_value`,
  `default_value_t`, and `default_values_t`.
- If any fields rely on clap string defaults together with OrthoConfig merge
  semantics, convert them to typed clap defaults and remove any mixed default
  override combinations that v0.8.0 now rejects.
- Keep or extend regression coverage so that CLI defaults still do not override
  higher-precedence environment or config values unexpectedly.

### 4. Revalidate YAML parsing behaviour

- Review configuration fixtures and docs that mention YAML parsing.
- Add or adjust tests for YAML 1.2 strictness if the repository already
  exercises YAML support, with explicit cases for quoted legacy literals where
  string semantics matter.
- Remove or rewrite any fixtures that rely on duplicate mapping keys being
  accepted.

### 5. Update repository documentation

- Update `README.md` to advertise `ortho-config` v0.8.0 instead of v0.6.0.
- Update `docs/ortho-config-users-guide.md` examples and dependency snippets to
  the new version.
- Either add a new migration guide for the v0.8.0 adoption or amend existing
  documentation so the current state of the repo and the upstream library
  behaviour are not contradictory.
- If documentation artifact generation is adopted, add the required
  `[package.metadata.ortho_config]` configuration and document the
  `cargo orthohelp` workflow. If it is intentionally deferred, record that
  decision in the docs to avoid ambiguity.

### 6. Run quality gates and confirm behaviour

- Run formatting, lint, and test gates after the upgrade.
- Run Markdown validation after documentation changes.
- Review any compile failures for macro-generated path changes first; those are
  the most likely breakage mode when moving between OrthoConfig releases.

## Validation checklist

Use the repository's required tee-and-pipefail pattern so failures are not
hidden by truncated output:

```bash
set -o pipefail && make fmt 2>&1 | tee /tmp/adopt-ortho-config-v0-8-0.fmt.log
set -o pipefail && make lint 2>&1 | tee /tmp/adopt-ortho-config-v0-8-0.lint.log
set -o pipefail && make test 2>&1 | tee /tmp/adopt-ortho-config-v0-8-0.test.log
set -o pipefail && make markdownlint 2>&1 | tee /tmp/adopt-ortho-config-v0-8-0.markdownlint.log
set -o pipefail && make nixie 2>&1 | tee /tmp/adopt-ortho-config-v0-8-0.nixie.log
```

During implementation, also perform these targeted checks:

- `cargo tree -i ortho_config`
- `rg -n "cli_default_as_absent|SelectedSubcommandMerge" .`
- `rg -n "default_value(_t|_values_t)?" .`
- `rg -n 'ortho_config\\(crate =' .`
- `rg -n "yes|on|off" tests src docs README.md`

## Risks and open questions

- v0.8.0 may tighten derive-generated paths or trait bounds in ways not obvious
  from the current surface scan; the first full compile will likely reveal the
  real work.
- This repo currently documents `ortho-config` usage heavily. The upgrade is
  not complete until those docs match the new dependency version and behaviour.
- If the project wants `cargo orthohelp` outputs, that is a separate scope
  choice with its own metadata and release-process implications. It should be
  decided explicitly rather than left half-configured.

## Definition of done

- `ortho_config` resolves to v0.8.0 everywhere in the workspace.
- The application builds and all tests and lint gates pass.
- Any required derive attributes, clap default adjustments, and YAML fixture
  changes are implemented.
- Repository documentation reflects the adopted version and any deliberate
  deferrals.

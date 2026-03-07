# Execplan: adopt `ortho-config` v0.8.0

## Goal

Upgrade this repository from `ortho-config` v0.6.0 to v0.8.0, preserving the
existing configuration precedence rules for `vk` while aligning the codebase,
tests, and documentation with the v0.8.0 migration requirements.

## Authoritative upstream inputs

- Replace the local OrthoConfig guide with the upstream v0.8.0 user guide from
  <https://raw.githubusercontent.com/leynos/ortho-config/refs/tags/v0.8.0/docs/users-guide.md>.
- Apply the v0.7.0 migration guidance from
  <https://raw.githubusercontent.com/leynos/ortho-config/refs/tags/v0.8.0/docs/v0-7-0-migration-guide.md>
  before closing the v0.8.0 migration work, because this repository is still on
  v0.6.x today.
- Apply the v0.8.0 migration notes for dependency versions, crate aliasing,
  clap defaults, YAML parsing, re-export usage, and optional `cargo orthohelp`
  metadata.

## Current state

- `Cargo.toml` pins `ortho_config = "0.6.0"` and forwards the optional `toml`,
  `json5`, and `yaml` features from the application crate.
- The application uses derive macros through `#[derive(OrthoConfig)]`, but does
  not declare `ortho_config_macros` directly in `Cargo.toml`.
- The code imports the runtime crate as `ortho_config` everywhere; no crate
  alias is currently in use.
- The repository already uses a recent nightly toolchain in
  `rust-toolchain.toml`, and `ortho_config` v0.8.0 requires Rust 1.89.0. The
  repository currently enforces that floor via `Cargo.toml`
  `rust-version = "1.89"` and also builds on the newer nightly declared in
  `rust-toolchain.toml`.
- Configuration merge behaviour is covered by integration tests in
  `tests/cli_args_merge.rs` and helpers under `tests/support/`.
- Documentation still advertises `ortho-config` v0.6.0 in `README.md` and in
  the OrthoConfig guides under `docs/`. The local
  `docs/ortho-config-users-guide.md` is not yet the upstream v0.8.0 guide.

## Migration notes mapped to this repo

### Applies directly

- Replace `docs/ortho-config-users-guide.md` with the upstream v0.8.0 guide,
  then reconcile any repo-local links, examples, or references that cannot be
  imported verbatim.
- Update the `ortho_config` dependency to `0.8.0`, then refresh `Cargo.lock`.
- Confirm that the active toolchain remains compatible with the new minimum
  Rust version.
- Review the additive v0.7.0 APIs and behaviours:
  `compose_layers()` / `compose_layers_from_iter(...)`,
  `PostMergeHook`, `FluentLocalizer`,
  `localize_clap_error_with_command`, and `cli_default_as_absent` flows that
  require `ArgMatches`.
- Audit YAML-backed tests and sample configuration for YAML 1.2 compatibility,
  especially any unquoted `yes`, `on`, or `off` literals and any duplicate
  mapping keys.
- Prefer `ortho_config` re-exports if implementation code or tests import
  `figment`, `uncased`, or `xdg` directly during the upgrade.
- Update project documentation that currently references v0.6.0 behaviour or
  contradicts the imported upstream v0.8.0 guide.

### Likely no-op, but must be verified

- Crate alias support: no `Cargo.toml` alias for `ortho_config` is currently
  present, so `#[ortho_config(crate = "...")]` should not be needed unless an
  alias appears in another manifest or test-only crate.
- `serde_json` under disabled defaults: the current dependency declaration does
  not disable default features, so the v0.7.0 requirement to enable
  `serde_json` explicitly should be a no-op unless the dependency configuration
  changes during implementation.
- `cli_default_as_absent`: no current use was found in the application code.
  Confirm this remains true across the full tree before implementation.
- Post-merge hooks and localisation: the v0.7.0 guide makes these available,
  but they are optional unless the repository chooses to adopt them for current
  `vk` behaviour or documentation examples.
- Documentation artifact generation: no existing use of `cargo orthohelp`,
  `OrthoConfigDocs`, or `[package.metadata.ortho_config]` was found. Decide
  whether this repository wants to add doc generation now or explicitly defer
  it.

## Implementation plan

### 1. Replace the local user guide with the upstream v0.8.0 guide

- Replace `docs/ortho-config-users-guide.md` with the exact upstream v0.8.0
  guide content from the tagged raw URL.
- Review the imported document for repo-specific mismatches:
  relative links, example paths, Mermaid diagrams, command references, and any
  assumptions about files that only exist in the upstream `ortho-config` repo.
- Decide whether to keep the document verbatim with upstream links, or adapt
  broken relative links to absolute GitHub URLs so the guide remains navigable
  from this repository.
- Reconcile any overlap with local migration guides so the documentation set is
  coherent after the replacement.

### 2. Prepare the dependency and toolchain update

- Change `ortho_config` in `Cargo.toml` from `0.6.0` to `0.8.0`.
- Run a lockfile refresh so the generated dependency graph matches v0.8.0.
- Inspect the resolved `ortho_config` and transitive parser crates in
  `Cargo.lock` to confirm the expected versions landed.
- Set `rust-version = "1.89"` in `Cargo.toml` to match the published
  `ortho_config` v0.8.0 minimum supported Rust version (MSRV). The nightly
  toolchain in `rust-toolchain.toml` exceeds that floor, but the manifest entry
  keeps the compatibility contract explicit for stable builds and tooling.

### 3. Reconcile the v0.7.0 and v0.8.0 API surface

- Search for code paths that would benefit from or depend on the v0.7.0
  additions:
  `compose_layers()`, `compose_layers_from_iter(...)`, `PostMergeHook`,
  `FluentLocalizer`, `localize_clap_error_with_command`,
  `SelectedSubcommandMerge`, and `cli_default_as_absent`.
- Where the repository manually inspects or synthesizes layers in tests,
  evaluate whether `compose_layers()` or `merge_from_layers()` should replace
  bespoke merge scaffolding as part of the upgrade.
- If any cross-field normalization or validation currently happens outside the
  merge pipeline, decide whether a `PostMergeHook` would simplify that logic.
- If CLI localisation is in scope for `vk`, capture that as an explicit follow-
  on decision rather than leaving the new v0.7.0 surface undocumented.

### 4. Reconcile derive and import changes

- Search for every `#[derive(... OrthoConfig ...)]` and any use of
  `SelectedSubcommandMerge` or similar derive-generated APIs.
- If any crate aliasing is introduced or discovered during the dependency
  update, add `#[ortho_config(crate = "...")]` to the affected derive sites.
- Search for direct imports of `figment`, `uncased`, or `xdg`; replace them
  with `ortho_config::figment`, `ortho_config::uncased`, and
  `ortho_config::xdg` where the source is derive-generated or tightly coupled
  to the runtime crate.

### 5. Audit clap default handling and selected-subcommand merges

- Search the full tree for `cli_default_as_absent`, `default_value`,
  `default_value_t`, and `default_values_t`.
- If any selected-subcommand merge flow is introduced or discovered, verify it
  passes `ArgMatches` through merge calls and adds
  `#[ortho_subcommand(with_matches)]` on variants that rely on clap default
  source tracking.
- If any fields rely on clap string defaults together with OrthoConfig merge
  semantics, convert them to typed clap defaults and remove any mixed default
  override combinations that v0.8.0 now rejects.
- Keep or extend regression coverage so that CLI defaults still do not override
  higher-precedence environment or config values unexpectedly.

### 6. Revalidate YAML parsing behaviour

- Review configuration fixtures and docs that mention YAML parsing.
- Add or adjust tests for YAML 1.2 strictness if the repository already
  exercises YAML support, with explicit cases for quoted legacy literals where
  string semantics matter.
- Remove or rewrite any fixtures that rely on duplicate mapping keys being
  accepted.

### 7. Update repository documentation around the imported guide

- Update `README.md` to advertise `ortho-config` v0.8.0 instead of v0.6.0.
- Add or update local migration documentation so the step from v0.6.x to
  v0.8.0 is covered, including the v0.7.0 migration notes that still matter to
  this repository.
- Ensure any local documentation that overlaps with the imported upstream guide
  either complements it or is removed to avoid contradictory advice.
- If documentation artifact generation is adopted, add the required
  `[package.metadata.ortho_config]` configuration and document the
  `cargo orthohelp` workflow. If it is intentionally deferred, record that
  decision in the docs to avoid ambiguity.

### 8. Run quality gates and confirm behaviour

- Run formatting, lint, and test gates after the upgrade.
- Run Markdown validation after documentation changes.
- Because the upstream v0.8.0 user guide contains a Mermaid diagram, ensure the
  repository's Mermaid validation step (`make nixie`) passes once the guide is
  imported.
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
- `diff -u docs/ortho-config-users-guide.md <downloaded-v0.8.0-guide>`
- `rg -n "cli_default_as_absent|SelectedSubcommandMerge" .`
- `rg -n "default_value(_t|_values_t)?" .`
- `rg -n 'ortho_config\\(crate =' .`
- `rg -n "compose_layers|compose_layers_from_iter|PostMergeHook" .`
- `rg -n "FluentLocalizer|localize_clap_error_with_command" .`
- `rg -n "yes|on|off" tests src docs README.md`

## Risks and open questions

- The upstream v0.8.0 user guide references `examples/hello_world` and other
  upstream-repo assets. Importing it verbatim may leave broken relative links
  inside this repository unless those references are rewritten.
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
- The local user guide is replaced with the upstream v0.8.0 guide, with any
  necessary link or reference fixes applied deliberately.
- Any required derive attributes, clap default adjustments, selected-subcommand
  merge updates, and YAML fixture changes are implemented.
- Repository documentation reflects the adopted version, the imported guide,
  and any deliberate deferrals.

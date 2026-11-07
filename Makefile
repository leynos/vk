.PHONY: help all clean test test-unstable-rest-resolve typecheck build release lint lint-clippy lint-whitaker fmt check-fmt markdownlint nixie test-workflow-contracts

APP ?= vk
CARGO ?= cargo
WHITAKER ?= whitaker
BUILD_JOBS ?=
# The build standard: every `rustflags` source in `.cargo/config.toml` carries
# the parallel frontend, and the Linux source adds `mold`. Assigning `RUSTFLAGS`
# replaces those sources outright, so the targets that assign it restate the
# flags here. The recipes add them to any inherited `RUSTFLAGS` (setup-rust
# exports one in CI) instead of replacing it. `make release` adds neither
# standard flag; coverage takes neither only when its caller exports
# `RUSTFLAGS`, as setup-rust does in CI.
STANDARD_THREADS_FLAG ?= -Zthreads=8
STANDARD_MOLD_FLAG ?= -Clink-arg=-fuse-ld=mold
BUILD_HOST_OS ?= $(shell uname -s)
# mold is added only when the machine doing the build is Linux (only Make can
# tell whether it has mold) and the compilation target is Linux too, which is
# the host unless `CARGO_BUILD_TARGET` names another triple. Android triples
# contain `-linux-` but report `target_os = "android"`, so they are not Linux.
STANDARD_TARGET_IS_LINUX = $(if $(CARGO_BUILD_TARGET),$(or $(filter host-tuple,$(CARGO_BUILD_TARGET)),$(and $(findstring -linux-,$(CARGO_BUILD_TARGET)),$(if $(findstring -android,$(CARGO_BUILD_TARGET)),,yes))),yes)
STANDARD_RUSTFLAGS = $(STANDARD_THREADS_FLAG)$(if $(filter Linux,$(BUILD_HOST_OS)),$(if $(STANDARD_TARGET_IS_LINUX), $(STANDARD_MOLD_FLAG)))
# Release builds add neither standard flag: assigning `RUSTFLAGS`, even to an
# empty inherited value, displaces every `rustflags` source in the
# configuration, and a caller's own value passes through untouched.
RELEASE_RUSTFLAGS = RUSTFLAGS="$${RUSTFLAGS-}"
# Debug builds keep a caller's exported flags and add the standard ones,
# since an inherited `RUSTFLAGS` would otherwise displace the configuration.
DEBUG_RUSTFLAGS = RUSTFLAGS="$${RUSTFLAGS:+$$RUSTFLAGS }$(STANDARD_RUSTFLAGS)"
# Gate targets also deny warnings.
GATE_RUSTFLAGS = RUSTFLAGS="$${RUSTFLAGS:+$$RUSTFLAGS }-D warnings $(STANDARD_RUSTFLAGS)"
CLIPPY_FLAGS ?= --all-targets --all-features -- -D warnings
RUSTDOC_FLAGS ?= --cfg docsrs -D warnings
MDLINT ?= markdownlint-cli2
# `make fmt` and `make check-fmt` call mdtablefix directly. `--git` selects the
# Markdown files Git tracks and `--include-untracked` adds the untracked files
# Git does not ignore, so a new document is formatted before it is staged.
# Both modes need mdtablefix 0.6.0 or later; CI pins the version at the
# install-mdtablefix step.
MDTABLEFIX ?= mdtablefix
MDTABLEFIX_SELECT = --git --include-untracked
MDTABLEFIX_RULES = --wrap --renumber --breaks --ellipsis --fences
NIXIE ?= nixie

UV ?= uv
UV_ENV = UV_CACHE_DIR=.uv-cache UV_TOOL_DIR=.uv-tools
# The CV-005 CodeScene contracts live in shared-actions and run from a full
# commit, so a fix is a pin bump. `.github/cv005.toml` holds this repository's
# only parameters.
CV005_CONTRACTS_REF ?= 88977798a5c3bae1549afb99642529488c665276
CV005_CONTRACTS = $(UV_ENV) $(UV) tool run --python 3.13 \
	--from 'git+https://github.com/leynos/shared-actions@$(CV005_CONTRACTS_REF)\#subdirectory=packages/cv005-contracts' \
	cv005-contracts

test-workflow-contracts: ## Check the CV-005 CodeScene workflow contracts
	$(CV005_CONTRACTS) check --repository .

build: target/debug/$(APP) ## Build debug binary
release: target/release/$(APP) ## Build release binary

all: release ## Default target builds release binary

clean: ## Remove build artifacts
	$(CARGO) clean

test: ## Run tests with warnings treated as errors
	$(CARGO) fetch --locked --target "$${CARGO_BUILD_TARGET:-host-tuple}"
	$(GATE_RUSTFLAGS) $(CARGO) test --all-targets --all-features $(BUILD_JOBS)

# The tests that exist only with `unstable-rest-resolve`: the `resolve`
# integration test, which is compiled out without the feature, and the unit
# tests under `resolve::rest`. Every other test is feature-independent and is
# run by the coverage step, so running `make test` here repeated them.
test-unstable-rest-resolve: ## Run only the tests the unstable-rest-resolve feature adds
	$(GATE_RUSTFLAGS) $(CARGO) test --features unstable-rest-resolve --test resolve $(BUILD_JOBS)
	$(GATE_RUSTFLAGS) $(CARGO) test --features unstable-rest-resolve --bin $(APP) resolve::rest $(BUILD_JOBS)

typecheck: ## Check all targets and features
	$(GATE_RUSTFLAGS) $(CARGO) check --all-targets --all-features $(BUILD_JOBS)

target/%/$(APP): Cargo.toml ## Build binary in debug or release mode
	$(if $(findstring release,$(@)),$(RELEASE_RUSTFLAGS),$(DEBUG_RUSTFLAGS)) $(CARGO) build $(BUILD_JOBS) $(if $(findstring release,$(@)),--release) --bin $(APP)

lint: lint-clippy lint-whitaker ## Run Clippy, rustdoc and the Whitaker suite with warnings denied

lint-clippy: ## Run Clippy and rustdoc with warnings denied
	$(GATE_RUSTFLAGS) $(CARGO) clippy $(CLIPPY_FLAGS)
	$(GATE_RUSTFLAGS) RUSTDOCFLAGS="$(RUSTDOC_FLAGS)" $(CARGO) doc --no-deps

WHITAKER_CLEAN_ENV = env -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
	-u CARGO_PROFILE_DEV_CODEGEN_BACKEND -u CARGO_BUILD_TARGET \
	-u CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER \
	-u CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS -u CFLAGS -u LDFLAGS

lint-whitaker: ## Run the Whitaker Dylint suite with clean driver inputs and warnings denied
	$(WHITAKER_CLEAN_ENV) RUSTFLAGS= DYLINT_RUSTFLAGS="-D warnings" $(WHITAKER) --all -- --all-targets --all-features

fmt: ## Format Rust and Markdown sources
	$(CARGO) fmt --all
	$(MDTABLEFIX) --in-place $(MDTABLEFIX_SELECT) $(MDTABLEFIX_RULES)
	@unset FORCE_COLOR; $(MDLINT) --fix "**/*.md"

check-fmt: ## Verify formatting
	$(CARGO) fmt --all -- --check
	$(MDTABLEFIX) --check $(MDTABLEFIX_SELECT) $(MDTABLEFIX_RULES)

markdownlint: ## Lint Markdown files
	find . -type f -name '*.md' -not -path '*/target/*' -not -path '*/node_modules/*' -print0 | xargs -0 $(MDLINT)

nixie: ## Validate Mermaid diagrams
	find . -type f -name '*.md' -not -path './target/*' -print0 | xargs -0 -- $(NIXIE)

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
	awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'

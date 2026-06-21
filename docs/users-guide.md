# User's guide

This guide is for people and agents using `vk` to inspect GitHub pull request
review comments from a terminal. It covers the stable user-facing workflows and
links to deeper reference material where needed.

## Install `vk`

Install the pre-built Linux release artefact with `cargo-binstall`:

```bash
cargo binstall vk
```

Release archives are published for `x86_64-unknown-linux-gnu` and
`aarch64-unknown-linux-gnu`. Other targets can build from source:

```bash
cargo install --path .
```

## Authenticate with GitHub

`vk` uses the GitHub GraphQL API. Provide a token using one of these sources,
listed from highest to lowest precedence:

- `--github-token <TOKEN>`
- `VK_GITHUB_TOKEN`
- `GITHUB_TOKEN`
- `github_token` in the active configuration file

The token needs read access for pull request and issue data. Use `public_repo`
for public repositories or `repo` for private repositories. See the
[GitHub token guide](github-token.md) for token creation and storage details.

## Configure default values

Configuration is powered by `ortho_config`. The most common global option is
the default repository, which lets numeric pull request references resolve to
`owner/repo`:

```bash
vk --repo leynos/vk pr 191
```

The same value can be supplied through `VK_REPO` or a configuration file:

```toml
repo = "leynos/vk"
```

Set `VK_CONFIG_PATH` or pass `--config-path` to choose a specific configuration
file. See the [Ortho Config users' guide](ortho-config-users-guide.md) for the
full configuration model.

## Inspect pull request comments

Show unresolved review comments for a pull request:

```bash
vk pr https://github.com/leynos/vk/pull/191
```

When a default repository is configured, a pull request number is enough:

```bash
vk pr 191
```

Add file paths after the pull request reference to show comments for those
files only:

```bash
vk pr 191 src/main.rs docs/users-guide.md
```

Outdated review threads are hidden by default. Include them with
`--show-outdated` or `-o`:

```bash
vk pr 191 --show-outdated
```

## Focus on one discussion

Pass a GitHub discussion fragment to show one review thread:

```bash
vk pr 191#discussion_r123456789
```

When a `#discussion_r<ID>` fragment is present, file filters are ignored and
both resolved and unresolved threads are searched so the specific discussion
can be found.

## Resolve a review thread

Resolve a pull request review thread with:

```bash
vk resolve https://github.com/leynos/vk/pull/191#discussion_r123456789
```

The resolver uses GitHub GraphQL. When `vk` is built with the
`unstable-rest-resolve` feature, `-m` or `--message` posts a reply before
resolving:

```bash
vk resolve https://github.com/leynos/vk/pull/191#discussion_r123456789 \
  --message "Addressed in the latest commit."
```

If the reply fails, the command stops before resolving the thread.

## Troubleshoot terminal output

`vk` renders comments with terminal Markdown and uses emoji to make output
easier to scan. If these render as unexpected characters, use a UTF-8 locale:

```bash
export LANG=C.UTF-8
```

`vk` prints a warning when it detects a non-UTF-8 locale.

## Related reference

- [README](../README.md): concise overview and quick examples.
- [GitHub token guide](github-token.md): token creation and storage.
- [Ortho Config users' guide](ortho-config-users-guide.md): configuration
  discovery, merge precedence, and file formats.
- [VK design](vk-design.md): behaviour rationale and architecture.

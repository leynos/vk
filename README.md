# View Komments (vk)

`vk` stands for **View Komments** because `vc` was already taken back in the
1970s, and no one argues with a greybeard. This command line tool fetches
unresolved GitHub code review comments for a pull request and displays them
with colourful terminal markdown using
[Termimad](https://crates.io/crates/termimad).

This tool is intended for use by AI coding agents such as Aider, OpenAI Codex
or Claude Code (without implying association with any of these companies).

## Usage

```bash
vk pr <pull-request-url-or-number> [FILE ...]
```

Specify file paths to show only comments for those files. Outdated threads are
hidden by default; pass `--show-outdated` (or `-o`) to include them.

Print the current version and exit with:

```bash
vk --version
```

`vk` now uses [OrthoConfig](https://github.com/leynos/ortho-config) v0.8.0 for
configuration. A global `--repo` option or the `VK_REPO` environment variable
sets the default repository when passing only a pull request number.

The CLI provides three subcommands:

- `pr` — show unresolved pull request comments. It begins with a
  `code review` banner (`========== code review ==========`), summarises files
  and comment counts, shows a `review comments` banner
  (`========== review comments ==========`) before individual threads, then
  prints an `end of code review` banner
  (`========== end of code review ==========`). Pass file paths after the pull
  request to restrict output to those paths. Use `--show-outdated` to include
  outdated threads.
- `issue` — read a GitHub issue (**to do**)
- `resolve` — resolve a pull request review thread. Accepts a comment
  reference (`#discussion_r<ID>` or full URL). Use `-m, --message <MESSAGE>` to
  post a reply before resolving (only when built with the
  `unstable-rest-resolve` feature).

When the feature is disabled, the message flag is ignored and only the GraphQL
resolution is performed.

`vk` loads default values for subcommands from configuration files and
environment variables. When these defaults omit the required `reference` field,
the tool continues with the value provided on the CLI instead of exiting with
an error.

If you pass just a pull request number, `vk` tries to work out which repository
you meant. It consults three sources in order:

1. the configured repository (`--repo` or `VK_REPO`), set to `owner/repo` with
   or without a `.git` suffix;
2. the GitHub URL recorded in `.git/FETCH_HEAD`, written by `git fetch`. In
   fork workflows this still points at the upstream repository where pull
   requests live, so it takes precedence over `origin`;
3. the URL of the `origin` remote, used as a last-resort fallback. This is
   what lets `vk pr` work in a fresh `git worktree add` target where
   `git fetch` has not yet been run inside the worktree.

If none of these resolve to a GitHub `owner/repo`, `vk` refuses to run with
only a number.

`vk` uses the GitHub GraphQL API. Set `GITHUB_TOKEN`, `VK_GITHUB_TOKEN`, pass
`--github-token`, or add `github_token` to `~/.config/vk/config.toml` (or the
path provided via `VK_CONFIG_PATH`) to authenticate. If no token is set, you'll
get a warning and anonymous requests may be rate limited. When
`VK_CONFIG_PATH` points at a file that cannot be parsed, `vk` exits early with
a `configuration error: …` message rather than silently falling back to
auto-discovered configuration.

The token only needs read access. Select the `public_repo` scope (or `repo` for
private repositories). See [docs/github-token.md](docs/github-token.md) for a
detailed guide to creating one.

## Example

```bash
vk pr https://github.com/leynos/mxd/pull/31
vk pr https://github.com/leynos/mxd/pull/31 src/main.rs
```

## Troubleshooting

`vk` renders comments with emoji for clarity. If these appear as random
characters, your terminal locale may not be UTF-8. Set `LC_ALL` or `LANG`
accordingly. `vk` prints a warning when it detects a non UTF-8 locale.

## Installing

Install the pre-built Linux release artifact with `cargo-binstall`:

```bash
cargo binstall vk
```

Release archives are published for:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

Other targets can still build from source using Cargo:

```bash
cargo install --path .
```

## License

This project is licensed under the ISC Licence. See [LICENSE](LICENSE) for
details.

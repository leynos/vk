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

`vk` now uses [OrthoConfig](https://github.com/leynos/ortho-config) v0.6.0 for
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
you meant. It first examines `.git/FETCH_HEAD` for a GitHub remote URL and, if
found, extracts the `owner/repo` from it. As the Codex agent does not put the
upstream URL in `.git/config`, we must obtain this from `FETCH_HEAD` for now.
Failing that, it falls back to the configured repository (`--repo` or
`VK_REPO`). Set this value to `owner/repo` with or without a `.git` suffix. If
neither source is available, `vk` will refuse to run with only a number.

`vk` uses the GitHub GraphQL API. Set `GITHUB_TOKEN`, `VK_GITHUB_TOKEN`, or add
`github_token` to `~/.config/vk/config.toml` (or the path provided via
`VK_CONFIG_PATH`) to authenticate. If no token is set, you'll get a warning and
anonymous requests may be rate limited.

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

Build from source using Cargo:

```bash
cargo install --path .
```

## License

This project is licensed under the ISC Licence. See [LICENSE](LICENSE) for
details.

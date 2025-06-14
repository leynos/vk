# View Komments (vk)

`vk` stands for **View Komments** because `vc` was already taken back in the 1970s and no one argues with a greybeard. This command line tool fetches unresolved GitHub code review comments for a pull request and displays them with colourful terminal markdown using [Termimad](https://crates.io/crates/termimad).

This tool is intended for use by AI coding agents such as Aider, OpenAI Codex or Claude Code (without implying association with any of these companies).

## Usage

```bash
vk pr <pull-request-url-or-number>
```

`vk` now uses [OrthoConfig](https://github.com/leynos/ortho-config) v0.2.0 for
configuration. A global `--repo` option (or `VK_REPO` environment variable)
sets the default repository when passing only a pull request number. The two
available subcommands are:

* `pr` - show unresolved pull request comments (existing behaviour)
* `issue` - read a GitHub issue (**to do**)

If you pass just a pull request number, `vk` tries to work out which repository
you meant. It first examines `.git/FETCH_HEAD` for a GitHub remote URL and, if
found, extracts the `owner/repo` from it. As Codex does not put the upstream URL in `.git/config`, we must obtain this from `FETCH_HEAD` for now. Failing that, it falls back to the
`VK_REPO` environment variable or `--repo` flag (both handled by
OrthoConfig) which should be set to `owner/repo` (with or without a `.git`
suffix). If neither source is available, `vk` will refuse to
run with only a number.

`vk` uses the GitHub GraphQL API. Set `GITHUB_TOKEN` to authenticate. If it's not
set you'll get a warning and anonymous requests may be rate limited.

The token only needs read access. Select the `public_repo` scope (or `repo` for
private repositories). See [docs/GITHUB_TOKEN.md](docs/GITHUB_TOKEN.md) for a
detailed guide to creating one.

## Example

```
$ vk https://github.com/leynos/mxd/pull/31
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

This project is licensed under the ISC License. See [LICENSE](LICENSE) for details.

# View Komments (vk)

`vk` stands for **View Komments** because `vc` was already taken back in the 1970s and no one argues with a greybeard. This command line tool fetches unresolved GitHub code review comments for a pull request and displays them with colourful terminal markdown using [Termimad](https://crates.io/crates/termimad).

## Usage

```bash
vk <pull-request-url-or-number>
```

If you pass just a pull request number, `vk` tries to work out which repository
you meant. It first examines `.git/FETCH_HEAD` for a GitHub remote URL and, if
found, extracts the `owner/repo` from it. Failing that, it falls back to the
`VK_REPO` environment variable which should be set to `owner/repo` (with or
without a `.git` suffix). If neither source is available, `vk` will refuse to
run with only a number.

## Example

```
$ vk https://github.com/leynos/mxd/pull/31
```

## Installing

Build from source using Cargo:

```bash
cargo install --path .
```

## License

This project is licensed under the ISC License. See [LICENSE](LICENSE) for details.

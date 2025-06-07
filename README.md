# View Komments (vk)

`vk` stands for **View Komments** because `vc` was already taken back in the 1970s and no one argues with a greybeard. This command line tool fetches unresolved GitHub code review comments for a pull request and displays them with colourful terminal markdown using [Termimad](https://crates.io/crates/termimad).

## Usage

```bash
vk <pull-request-url-or-number>
```

`vk` uses the GitHub GraphQL API and requires a GitHub token for API access, which should be provided in the `GITHUB_TOKEN` environment variable. If `GITHUB_TOKEN` is not set, you'll get a warning and anonymous requests may be rate limited.

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

# View Komments (vk)

`vk` stands for **View Komments** because `vc` was already taken back in the 1970s and no one argues with a greybeard. This command line tool fetches unresolved GitHub code review comments for a pull request and displays them with colourful terminal markdown using [Termimad](https://crates.io/crates/termimad).

## Usage

```bash
vk <pull-request-url-or-number>
```

`vk` uses the GitHub GraphQL API. Set `GITHUB_TOKEN` to authenticate. If it's not
set you'll get a warning and anonymous requests may be rate limited.

A GitHub token is required for API access and should be provided in `GITHUB_TOKEN`.

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

# View Komments (vk)

`vk` stands for **View Komments** because `vc` was already taken back in the 1970s and no one argues with a greybeard. This command line tool fetches unresolved GitHub code review comments for a pull request and displays them with colourful terminal markdown using [Termimad](https://crates.io/crates/termimad).

## Usage

```bash
vk <pull-request-url-or-number>
```

If you pass just a number, `vk` looks for the repository URL in `.git/FETCH_HEAD` first. If that fails it checks the `VK_REPO` environment variable. If neither is present `vk` will error.

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

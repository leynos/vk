# Creating a GitHub Token for vk

vk authenticates to the GitHub GraphQL API using a personal access token (PAT).
Follow these steps to create one:

1. Visit <https://github.com/settings/tokens> and choose **Generate new token**.
   GitHub may prompt for a classic or fine‑grained token – either works.
2. Give the token a note and set an expiration.
3. Under **Select scopes**, enable `public_repo`. If private repositories are
   required, select the broader `repo` scope instead.
4. Click **Generate token** and copy the value.
5. Export the token as `VK_GITHUB_TOKEN` (or `GITHUB_TOKEN`):

```bash
export GITHUB_TOKEN=YOUR_TOKEN
```

   Alternatively, store it in `~/.config/vk/config.toml` (or a file referenced
   by `VK_CONFIG_PATH`):

```toml
github_token = "YOUR_TOKEN"
```

   The token can also be passed directly with `--github-token` when running
   `vk`.

Token precedence (highest to lowest):

- `--github-token`
- `VK_GITHUB_TOKEN`
- `GITHUB_TOKEN`
- `~/.config/vk/config.toml` (or the file referenced by
  `VK_CONFIG_PATH`)

Once set, `vk` can run normally:

```bash
vk <pull-request-url-or-number>
```

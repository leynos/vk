# Creating a GitHub Token for vk

vk authenticates to the GitHub GraphQL API using a personal access token (PAT).
Follow these steps to create one:

1. Visit <https://github.com/settings/tokens> and choose **Generate new token**.
   GitHub may prompt you to pick a classic or fine‑grained token – either works.
2. Give the token a note and set an expiration.
3. Under **Select scopes**, enable `public_repo`. If you need to access private
   repositories, select the broader `repo` scope instead.
4. Click **Generate token** and copy the value.
5. Export the token as `GITHUB_TOKEN`:

```bash
export GITHUB_TOKEN=YOUR_TOKEN
```

Once set you can run `vk` normally:

```bash
vk <pull-request-url-or-number>
```

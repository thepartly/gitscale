# 2.7 CI authentication

- [The problem](#the-problem)
- [What GitScale does](#what-gitscale-does)
- [Detection](#detection)
- [What is never touched](#what-is-never-touched)
- [GitLab job token allowlists](#gitlab-job-token-allowlists)
- [GitHub Actions token scope](#github-actions-token-scope)
- [Turning it off](#turning-it-off)

## The problem

A CI runner has a short-lived token for the forge it runs on, but no SSH key. An
entry declared as

```toml
"imports/payments" = { url = "git@gitlab.example.com:acme/payments.git", revision = "main" }
```

clones fine on a laptop and fails inside a job.

The usual workaround is to rewrite URLs in the pipeline with
`git config --global url.…insteadOf`. GitScale derives the fix from the job
environment instead, so there is nothing to add to `.gitlab-ci.yml` or a
workflow file.

## What GitScale does

Inside a recognised job, **every entry hosted on that same server** is fetched
over HTTPS with the job token:

```
git@gitlab.example.com:acme/payments.git
    → https://gitlab.example.com/acme/payments.git   authenticated as gitlab-ci-token
```

Both SSH spellings are recognised — `git@host:group/repo.git` and
`ssh://git@host/group/repo.git` — and nested subgroups are preserved. An SSH
port is dropped, since it says nothing about the HTTPS endpoint. A URL already
spelled with the CI server's scheme and host is used as-is.

This applies to clones, fetches, pulls, pushes and the [object
cache](caching.md)'s own entries. An existing clone whose `origin` still points
at SSH — a workspace restored from a cache, or the runner's own checkout — is
repointed before the next network operation.

## Detection

| Forge | Required | Server URL from | Username |
|---|---|---|---|
| GitLab | `CI_JOB_TOKEN` | `CI_SERVER_URL`, or `CI_SERVER_HOST` + `CI_SERVER_PROTOCOL` + `CI_SERVER_PORT` | `gitlab-ci-token` |
| GitHub Actions | `GITHUB_ACTIONS` **and** `GITHUB_TOKEN` or `GH_TOKEN` | `GITHUB_SERVER_URL`, default `https://github.com` | `x-access-token` |

GitLab is checked first. `CI_JOB_TOKEN` exists only inside a job, so it doubles
as the detector — `GITLAB_CI` is deliberately not required, because a job running
GitScale in a nested container may forward the token without the rest of the
`CI_*` set. GitHub does require its runner marker, because `GITHUB_TOKEN` and
`GH_TOKEN` are commonly exported on developer machines and rewriting SSH remotes
during ordinary local use would be a surprise.

Only `http` and `https` server URLs are accepted; a port in the server URL is
kept.

Note that this is separate from `CI=1`, which is what switches GitScale to
[shallow clones and snapshot cache entries](caching.md#what-changes-in-ci).

## What is never touched

- **Other hosts are never offered the token.** The entry's host has to match the
  CI server's for the credential to apply at all; a dependency on a different
  server is fetched exactly as configured.
- **The token never lands on disk or in argv.** GitScale passes git a credential
  helper scoped to the CI server that reads the token variable when git asks for
  the password, so only the *name* of the variable appears on the command line.
  Any credential helper inherited from user or system config is reset for that
  host first.
- **Nothing sensitive is persisted.** The credential setup lives on the git
  command line for the duration of the call. The only thing written to
  `.git/config` is the plain HTTPS remote URL, which carries no credentials.

## GitLab job token allowlists

Detection cannot grant access. On GitLab, the target project must list the
calling project under **Settings → CI/CD → Job token permissions**, or the clone
returns 403 no matter how it authenticates. GitScale recognises that answer and
says so:

```
FAIL  imports/payments: fatal: unable to access 'https://gitlab.example.com/acme/payments.git/': The requested URL returned error: 403
hint: the GitLab job token (CI_JOB_TOKEN) was rejected. Add this project to the target project's Settings -> CI/CD -> 'Job token permissions' allowlist, or give the job a token with read access.
```

## GitHub Actions token scope

GitHub has no equivalent of the allowlist. The built-in `GITHUB_TOKEN` is a
freshly minted installation token for the GitHub Actions app, and that
installation is scoped to exactly **one repository** — the one the workflow
lives in. It expires when the job ends.

| Entry | Result |
|---|---|
| The workflow's own repository | works |
| Any public repository | works (readable without credentials anyway) |
| A **private** repository, even in the same org | 403 |

The workflow's `permissions:` block only widens or narrows *which scopes* the
token holds on its own repository (`contents`, `packages`, `id-token`, …). It
cannot extend the token to a second repository. `contents: read` is simply the
scope a clone needs.

For a private cross-repo dependency, supply a token that does cover it:

| Option | Notes |
|---|---|
| GitHub App token | Install an app on both repositories and mint a short-lived token in the job. No user account involved; expires within the hour |
| Fine-grained PAT | Grant **Contents: Read** on the specific repositories, store as a repo or org secret. Tied to a user account |
| Deploy key | An SSH key registered on the target repository. Per-repo, and keeps you on SSH rather than HTTPS |

GitScale reads whichever token is in `GITHUB_TOKEN` or `GH_TOKEN` and does not
care where it came from, so exporting a better token under that name is the
whole change:

```yaml
- uses: actions/create-github-app-token@v1
  id: app-token
  with:
    app-id: ${{ vars.APP_ID }}
    private-key: ${{ secrets.APP_PRIVATE_KEY }}
    owner: acme
- run: gitscale sync
  env:
    GITHUB_TOKEN: ${{ steps.app-token.outputs.token }}
```

## Turning it off

```
export GITSCALE_NO_CI_AUTH=1
```

GitScale then fetches exactly what the config says, with no rewriting and no
credential helper.

---

[← 2.6 Hooks](hooks.md) · [Contents](README.md) · [Next → 2.8 Cleaning](clean.md)

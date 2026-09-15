# 3. Configuration file reference

- [Where the file lives](#where-the-file-lives)
- [A complete example](#a-complete-example)
- [`[repos]`](#repos)
- [`[storage]`](#storage)
- [`[cache]`](#cache)
- [`[share]`](#share)
- [`[clean]`](#clean)
- [`[hooks]`](#hooks)
- [Nested configs](#nested-configs)
- [Values GitScale refuses to pass to git](#values-gitscale-refuses-to-pass-to-git)
- [How `add` and `remove` rewrite the file](#how-add-and-remove-rewrite-the-file)
- [Environment variables](#environment-variables)

## Where the file lives

`.gitscale.toml`, at the root of the workspace. Every command searches upward
from the current directory until it finds one; `-C, --root PATH` starts the
search from `PATH` instead.

A checked-out sub-repository may carry its own `.gitscale.toml`. Which parts of
it are read, and when, is covered under [nested configs](#nested-configs).

Unknown keys and unknown tables are ignored on read — but see
[how `add` and `remove` rewrite the file](#how-add-and-remove-rewrite-the-file).

## A complete example

```toml
[repos]
"imports/core"  = { url = "git@github.com:org/core.git", revision = "main", mode = "readonly" }
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
"meta/frontend" = { url = "https://github.com/org/frontend.git", revision = "main", mode = "artefact" }
"vendor/tools"  = { url = "https://github.com/org/tools.git", recursive = false }

[storage]
url = "https://my-bucket.s3.us-east-1.amazonaws.com/gitscale"

[cache]
enabled    = true
dir        = "~/.local/share/gitscale"
dissociate = false
adopt_root = false

[share]
dissociate = false

[clean]
exclude = [".vscode", ".idea", ".env", "envs/", "tmp"]

[hooks]
post_sync     = "make install"
on_pull_error = "warn"
```

Every table is optional. A config with nothing but `[clean]` in it is valid and
useful — see [per-repo `[clean]`](clean.md#per-repo-clean).

## `[repos]`

A table of entries, keyed by the directory the checkout lands in, relative to
the config.

```toml
[repos]
"imports/core" = { url = "git@github.com:org/core.git", revision = "main", mode = "readonly", recursive = true }
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `url` | string | **required** | Repository URL: HTTPS, SSH (`git@host:owner/repo.git` or `ssh://git@host/owner/repo.git`), or a local path |
| `revision` | string | `""` | Branch, tag or commit SHA. Empty means the remote's default branch, or a revision [adopted from a child config](recursive-dependencies.md#revision-resolution-and-the-mismatch-check). See [pinning a revision](dependencies.md#pinning-a-revision) |
| `mode` | string | `"readwrite"` | `"readwrite"`, `"readonly"` or `"artefact"`. See [checkout modes](dependencies.md#checkout-modes) |
| `recursive` | bool | `true` | Read this repository's own `.gitscale.toml`: resolve its transitive dependencies, and clean it by its own `[clean]` rules. With `false`, that config is not read at all and [`clean`](clean.md) skips the repository |

The directory key must be relative and free of `..`, and must not be empty.

## `[storage]`

Where [artefact](dependencies.md#artefact) archives live.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `url` | string | — | Base URL for artefact objects. **Required if the table is present**, and must not be empty |

```toml
[storage]
url = "https://my-bucket.s3.us-east-1.amazonaws.com/gitscale"
```

Backends, credentials and the object layout are covered in
[artefact storage](dependencies.md#artefact-storage). Without this table,
artefact entries fail with `no [storage] configured`; git entries are
unaffected.

## `[cache]`

The [object cache](caching.md).

| Key | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `true` | Use the object cache. `false` makes every clone and fetch talk to the remote directly, as `--no-cache` does per command |
| `dir` | string | `""` | Where entries live. Empty means the default location — see [where the cache lives](caching.md#where-the-cache-lives). A leading `~/` is expanded |
| `dissociate` | bool | `false` | Copy objects borrowed from the cache into each workspace and drop the link. Costs the disk saving, keeps the network one — see [dissociate](caching.md#copying-instead-of-borrowing-dissociate) |
| `adopt_root` | bool | `false` | Relink a workspace root cloned by plain `git clone` to the cache, reclaiming its duplicate objects — see [adopting a root repository](caching.md#adopting-a-root-repository) |

The cache is per-user by design; there is no system-wide scope and no key to
create one.

## `[share]`

How a clone may reuse a copy of a sub-repository already on this machine.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `dissociate` | bool | `false` | Copy objects borrowed from a [source workspace](caching.md#where-objects-come-from) in and drop the link once the clone is made |

Off by default: borrowing is what saves the disk, and the workspace borrowed
from is normally the long-lived one. Turn it on where the source may be pruned,
moved or garbage-collected out from under the clones — `git gc` there can delete
objects only a borrower still needs, and nothing warns when it does.

## `[clean]`

Which untracked files [`gitscale clean`](clean.md) keeps in **this** repository.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `exclude` | array of strings | `[]` | `.gitignore`-syntax patterns, anchored at this repository's root, passed to `git clean -e` unchanged |

Scoped to the repository whose config it appears in, and nothing below it. A
pattern may not be empty or start with `-`.

## `[hooks]`

Commands GitScale runs after its own operations. Not to be confused with
[git hooks](hooks.md#git-hooks), which is how GitScale hooks *into* git.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `post_sync` | string | — | Shell command run after `pull` and `sync` complete, via `sh -c` in the config root. A non-zero exit fails the command |
| `on_pull_error` | string | `"fail"` under CI, `"warn"` otherwise | What a **git-hook-triggered** pull does when it fails: `"fail"` returns non-zero and fails the git operation, `"warn"` reports and lets it succeed |

Under an installed [git hook](hooks.md#git-hooks), `post_sync` runs only for
repositories on that hook's [allowlist](hooks.md#the-hook-allowlist).

## Nested configs

When a checked-out repository carries its own `.gitscale.toml` and the entry is
`recursive = true` (the default), GitScale reads two things from it:

- **`[repos]`** — to resolve [transitive dependencies](recursive-dependencies.md):
  every entry must also be declared at the root, and a symlink is created
  instead of a nested checkout.
- **`[clean]`** — to decide what [`clean`](clean.md) keeps in that repository.

Everything else in a nested config — `[storage]`, `[cache]`, `[share]`,
`[hooks]` — belongs to that repository when it is used as a workspace in its own
right, and is not read by the parent.

## Values GitScale refuses to pass to git

`.gitscale.toml` travels inside the repository, so every value in it arrives from
whoever wrote the checked-out branch. Git treats some strings as instructions
rather than data, so these are rejected when the config is loaded, by every
command:

| Rejected | Why |
|---|---|
| `url = "ext::sh -c '…'"` and any `helper::` prefix | A remote helper makes git run the rest of the URL as a command — code execution from a repo URL alone, with no `[hooks]` table in sight. A prefix only counts as a helper when it is a bare word, so an IPv6 literal or a URL that merely contains `::` is left alone |
| A `url` or `revision` starting with `-` | It reaches git as a flag, and flags such as `--upload-pack=` name a program to run |
| A repo directory that is absolute or contains `..` | A config must not be able to decide to check out over `~/.ssh` |
| A `cache.dir` starting with `-` | It reaches git as a path argument, where a flag would be a shell in disguise |
| A `clean.exclude` pattern that is empty or starts with `-` | It reaches `git clean -e` as an argument |

These checks are independent of the [hook allowlist](hooks.md#the-hook-allowlist),
which covers the `[hooks]` table, and apply even to a `gitscale pull` you typed
yourself.

## How `add` and `remove` rewrite the file

`gitscale add` and `gitscale remove` re-emit `.gitscale.toml` from what GitScale
parsed. Consequences:

- Comments, blank lines, key order and formatting are lost.
- Any table GitScale does not know about is **deleted**.
- Defaults are not written back: `mode = "readwrite"`, `recursive = true`, an
  empty revision, and every default `[cache]` / `[share]` / `[clean]` / `[hooks]`
  value are simply omitted.
- Tables are emitted in a fixed order: `[share]`, `[cache]`, `[storage]`,
  `[hooks]`, `[clean]`, `[repos]`, with entries inline and sorted by directory.

If you keep comments in the file, edit it by hand instead.

## Environment variables

### Read by GitScale

| Variable | Effect |
|---|---|
| `CI` | `1` or `true` switches to [CI behaviour](caching.md#what-changes-in-ci): shallow checkouts and snapshot cache entries |
| `GITSCALE_CACHE_DIR` | Cache location, when `[cache] dir` is unset |
| `XDG_DATA_HOME` | `$XDG_DATA_HOME/gitscale` is the cache location, when neither of the above is set |
| `HOME` | `~/.local/share/gitscale` is the last fallback; also where `--global` hooks are installed |
| `GITSCALE_NO_CI_AUTH` | Any non-empty value disables [CI authentication](ci-authentication.md) |
| `GITSCALE_HOOK_ALLOW` | Set by an installed [git hook shim](hooks.md#the-hook-allowlist) to the allowlist it was installed with. Not something to set yourself |
| `GITSCALE_HOOK` | Set by GitScale on every git call it makes, so an installed hook can tell re-entry from a genuine user operation |

### CI authentication

| Variable | Forge |
|---|---|
| `CI_JOB_TOKEN` | GitLab — presence detects a job |
| `CI_SERVER_URL`, or `CI_SERVER_HOST` + `CI_SERVER_PROTOCOL` + `CI_SERVER_PORT` | GitLab — the server to authenticate to |
| `GITHUB_ACTIONS` | GitHub — required runner marker |
| `GITHUB_TOKEN`, `GH_TOKEN` | GitHub — the token, first one found |
| `GITHUB_SERVER_URL` | GitHub — the server, default `https://github.com` |

### Artefact storage

| Variable | Backend |
|---|---|
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | S3-compatible; required |
| `AWS_DEFAULT_REGION` | S3-compatible; otherwise derived from the URL, else `us-east-1` |
| `GOOGLE_TOKEN`, `GCLOUD_ACCESS_TOKEN` | Google Cloud Storage, first one found |

### Set by GitScale for git

Every git call GitScale makes runs with `GIT_TERMINAL_PROMPT=0`, an empty
`GIT_ASKPASS` and `SSH_ASKPASS`, and `SSH_ASKPASS_REQUIRE=never`, with stdin
closed. GitScale never prompts for credentials: a repository that needs
credentials git does not already have fails rather than hanging.

---

[← 2.8 Cleaning](clean.md) · [Contents](README.md) · [Next → 4. Command line reference](cli.md)

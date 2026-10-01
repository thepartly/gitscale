# 3. Configuration file reference

- [Where the file lives](#where-the-file-lives)
- [A complete example](#a-complete-example)
- [`[repos]`](#repos)
- [`[resolve]`](#resolve)
- [`singleton`](#singleton)
- [`[registries]`](#registries)
- [`[artefact]`](#artefact)
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

Unknown keys and unknown tables are ignored on read, except inside
[`[artefact]`](#artefact) and [`[resolve]`](#resolve), where a misspelt key would quietly publish less than
meant — but see
[how `add` and `remove` rewrite the file](#how-add-and-remove-rewrite-the-file).

## A complete example

```toml
[repos]
"imports/core"  = { url = "git@github.com:org/core.git", revision = "main", mode = "readonly" }
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
"meta/frontend" = { url = "https://github.com/org/frontend.git", revision = "main", mode = "artefact" }
"vendor/tools"  = { url = "https://github.com/org/tools.git", recursive = false }

[registries]
"gitlab.corp.example" = "registry.corp.example"

[artefact]
root    = "dist"
include = ["**"]
exclude = ["**/*.map"]

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
| `revision` | string | `""` | Branch, tag or full commit SHA (40 or 64 hex digits). A minimum: [resolution](recursive-dependencies.md#how-a-revision-is-chosen) may raise it to what a dependency asks for. Empty asks for nothing, leaving it to the dependencies — or, when nobody asks, the remote's default branch. See [pinning a revision](dependencies.md#pinning-a-revision) |
| `mode` | string | `"readwrite"` | `"readwrite"`, `"readonly"` or `"artefact"`. See [checkout modes](dependencies.md#checkout-modes) |
| `recursive` | bool | `true` | Read this repository's own `.gitscale.toml`: resolve its transitive dependencies, and clean it by its own `[clean]` rules. With `false`, that config is not read at all, and [`clean`](clean.md#per-repo-clean) cleans the repository without its keep-list |
| `override` | bool | `false` | Exactly this revision, and nothing higher: wins over every request from a repository below this one, and must agree with the rest. Needs a `revision`. See [overrides](recursive-dependencies.md#overrides) |
| `singleton` | bool | unset | `true`: this repository may be checked out only once, whatever majors are asked for. `false`: relaxes a `true` from repositories below this one. See [singleton](recursive-dependencies.md#singleton) |

The directory key must be relative and free of `..`, and must not be empty.
Two entries for one repository need revisions to tell which major each is for.

## `[resolve]`

How dependencies the root does not declare are
[checked out](recursive-dependencies.md#implicit-dependencies). Read from the
root only.

```toml
[resolve]
hoist_dir = "imports"
allow = ["github.com/partner-org/*", "gitlab.example.com/platform/*"]
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `hoist_dir` | string | `"imports"` | Where implicit checkouts go, relative to the config. Must stay inside the workspace |
| `allow` | list of strings | `[]` | Patterns for repositories an implicit dependency may come from, on top of the host and owner of every repository the root declares. Globs over `host/owner/repo`, as [`hook install --allow`](hooks.md) takes them; `*` crosses `/`, so `github.com/org/*` covers nested groups. A local path is allowed only by a pattern here |

## `singleton`

```toml
singleton = true

[repos]
# …
```

A top-level key, before any table: this repository says of itself that a
workspace may hold only one checkout of it. Read from a dependency's config at
the revision selected. See [singleton](recursive-dependencies.md#singleton).

## `[registries]`

Where [artefacts](artefacts.md) are published, for repositories the built-in
mapping does not cover. GitHub, GitLab.com and the registry a CI job's own
server owns need nothing here — see
[where artefacts live](artefacts.md#where-artefacts-live).

```toml
[registries]
"gitlab.corp.example" = "registry.corp.example"
"/srv/repos/"         = "registry.corp.example:5000/mirrors"
```

| Key | Value |
|---|---|
| a host, such as `gitlab.corp.example` | The registry for every repository on that host, as `host[:port][/namespace]`. The image is `<registry>/<repo path>/gitscale` |
| anything containing `/`: a URL prefix | The registry for every repository URL starting with it, on a path boundary. The longest matching prefix wins, and the image path is what follows the prefix |

Both kinds of key and value must be non-empty. A value carries no scheme,
except `http://` to opt in to plain HTTP for a registry without TLS. A registry
on `localhost`, `127.0.0.1` or `[::1]` is spoken to over plain HTTP anyway,
every other one over HTTPS. No credential is sent over plain HTTP to anything
but this machine.

The table is read by consumers and by [`artefact publish`](artefacts.md#artefact-publish)
alike, so a producer and its consumers on a self-hosted forge need the same
entry.

The `[storage]` table, whose `url` named an S3, GCS or local bucket, is no
longer supported: a config that has one fails to load, with directions.

## `[artefact]`

What [`gitscale artefact publish`](artefacts.md#artefact-publish) ships from
this repository: its build output, as one image layer per group of glob
patterns.

```toml
[artefact]
root = "dist"

[[artefact.layer]]
name    = "vendor"
include = ["vendor/**"]

[[artefact.layer]]
name    = "app"
include = ["**"]
exclude = ["**/*.map"]
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `root` | string | `"."` | The directory, relative to the config, that patterns and the archive's paths are relative to. Must stay below the config's directory |
| `include` | array of strings | — | For a single group: the files to ship |
| `exclude` | array of strings | `[]` | For a single group: files to leave out of what `include` matched |
| `layer` | array of tables | — | One table per group, in layer order, each with `name`, `include` and an optional `exclude`. Use either these or `include` directly, not both |

A layer's `name` is letters, digits, `.`, `_` or `-`, unique, and recorded as
the layer's title. Patterns are [plain globs](artefacts.md#patterns) relative to
`root`; one that starts with `/`, contains `.` or `..` components, or does not
compile is an error when the config is read. A file goes to the first group
that matches it, and a group that matches nothing fails the publish.

This table belongs to the producing repository. A workspace that declares an
artefact entry never reads it.

## `[cache]`

The [object cache](caching.md).

| Key | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `true` | Use the object cache. `false` makes every clone and fetch talk to the remote directly, as `--no-cache` does per command |
| `dir` | string | `""` | Where entries live. Empty means the default location — see [where the cache lives](caching.md#where-the-cache-lives). A leading `~/` is expanded |
| `dissociate` | bool | `false` | Copy objects borrowed from the cache into each workspace and drop the link. Costs the disk saving, keeps the network one — see [dissociate](caching.md#copying-instead-of-borrowing-dissociate) |
| `adopt_root` | bool | `false` | Relink a workspace root cloned by plain `git clone` to the cache, reclaiming its duplicate objects. Skipped for shallow clones and linked worktrees; [`cache adopt`](caching.md#cache-adopt) does it on demand — see [adopting a root repository](caching.md#adopting-a-root-repository) |

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
`recursive = true` (the default), GitScale reads three things from it:

- **`[repos]`** — to resolve [transitive dependencies](recursive-dependencies.md):
  every entry is a request, resolved with the rest of the workspace, checked
  out once and linked instead of nested.
- **`singleton`** — whether the repository allows only one checkout of itself.
- **`[clean]`** — to decide what [`clean`](clean.md) keeps in that repository.

Everything else in a nested config — `[resolve]`, `[registries]`, `[artefact]`,
`[cache]`, `[share]`, `[hooks]` — is not even parsed by the parent, so nothing
in those tables can break it. It belongs to that repository when it is used as a workspace in its own
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
- Tables are emitted in a fixed order: a top-level `singleton`, `[resolve]`,
  `[share]`, `[cache]`, `[registries]`, `[artefact]`, `[hooks]`, `[clean]`,
  `[repos]`, with
  entries inline and sorted by directory. A single `[artefact]` group named
  `default` is written in the short form, with `include` directly in the table.

If you keep comments in the file, edit it by hand instead.
[`gitscale resolve --write`](cli.md#gitscale-resolve) is the exception: it changes only
the revisions it reports, and keeps everything else as it was.

## Environment variables

### Read by GitScale

| Variable | Effect |
|---|---|
| `CI` | `1` or `true` switches to [CI behaviour](caching.md#what-changes-in-ci): shallow checkouts, snapshot cache entries, and [`clean -f` on every checkout](hooks.md#git-hooks-in-ci) after a `pull` |
| `GITSCALE_CACHE_DIR` | Cache location, when `[cache] dir` is unset |
| `XDG_DATA_HOME` | `$XDG_DATA_HOME/gitscale` is the cache location, when neither of the above is set |
| `HOME` | `~/.local/share/gitscale` is the last fallback; also where `--global` hooks are installed |
| `GITSCALE_NO_CI_AUTH` | Any non-empty value disables [CI authentication](ci-authentication.md) |
| `GITSCALE_HOOK_ALLOW` | Set by an installed [git hook shim](hooks.md#the-hook-allowlist) to the allowlist it was installed with. Not something to set yourself |
| `GITSCALE_HOOK` | Set by GitScale on every git call it makes, so an installed hook can tell re-entry from a genuine user operation |
| `GITLAB_CI` | `true` makes a hook-triggered pull check that the runner's post-checkout clean keeps the declared checkouts, and [fail if it would not](hooks.md#git-hooks-in-ci) |
| `GIT_CLEAN_FLAGS` | GitLab Runner's own: the flags of the `git clean` it runs after its checkout, default `-ffdx`. Read for that check, never set by GitScale |

### CI authentication

| Variable | Forge |
|---|---|
| `CI_JOB_TOKEN` | GitLab — presence detects a job |
| `CI_SERVER_URL`, or `CI_SERVER_HOST` + `CI_SERVER_PROTOCOL` + `CI_SERVER_PORT` | GitLab — the server to authenticate to |
| `GITHUB_ACTIONS` | GitHub — required runner marker |
| `GITHUB_TOKEN`, `GH_TOKEN` | GitHub — the token, first one found |
| `GITHUB_SERVER_URL` | GitHub — the server, default `https://github.com` |

### Artefact registries

| Variable | Effect |
|---|---|
| `CI_REGISTRY` | GitLab — the registry the CI server owns: the only one the job token is sent to, and where repositories on the CI server's own host publish |
| `CI_COMMIT_SHA`, `CI_PROJECT_URL` | GitLab, with `GITLAB_CI=true` — the commit and repository `artefact publish` publishes for |
| `GITHUB_SHA`, `GITHUB_REPOSITORY` | GitHub Actions — the same |
| `DOCKER_CONFIG` | The directory holding Docker's `config.json`, instead of `~/.docker` — for [stored logins](artefacts.md#logging-in) |
| `REGISTRY_AUTH_FILE` | Podman's auth file, instead of `$XDG_RUNTIME_DIR/containers/auth.json` and `~/.config/containers/auth.json` |

### Set by GitScale for git

Every git call GitScale makes runs with `GIT_TERMINAL_PROMPT=0`, an empty
`GIT_ASKPASS` and `SSH_ASKPASS`, and `SSH_ASKPASS_REQUIRE=never`, with stdin
closed. GitScale never prompts for credentials: a repository that needs
credentials git does not already have fails rather than hanging.

That includes the passphrase of an ssh key: a key that has one must be loaded
into an ssh agent (`ssh-add`). On a machine you reach over SSH, forwarding the
agent of the machine you connect from (`ForwardAgent yes`) works too. When ssh
refuses the key, GitScale checks the agent and prints a `hint:` saying which of
these is missing, once for the whole run.

---

[← 2.9 Artefacts](artefacts.md) · [Contents](README.md) · [Next → 4. Command line reference](cli.md)

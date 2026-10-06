# 3. Configuration file reference

- [Where the file lives](#where-the-file-lives)
- [A complete example](#a-complete-example)
- [`[repos]`](#repos)
- [`[resolve]`](#resolve)
- [`singleton`](#singleton)
- [`[registries]`](#registries)
- [`[artefact]`](#artefact)
- [`[branches]`](#branches)
- [`[topic]`](#topic)
- [`[clean]`](#clean)
- [`[hooks]`](#hooks)
- [`[forward]`](#forward)
- [Nested configs](#nested-configs)
- [Values GitScale refuses to pass to git](#values-gitscale-refuses-to-pass-to-git)
- [Git config keys](#git-config-keys)
- [Environment variables](#environment-variables)

## Where the file lives

`.gitscale.toml`, at the top of the workspace's root repository — anywhere
else, GitScale refuses. Every command searches upward from the current
directory, past any checkout GitScale made, to the workspace root — see
[finding the workspace](cli.md#finding-the-workspace); `-C, --root PATH` starts
the search from `PATH` instead.

A checked-out sub-repository may carry its own `.gitscale.toml`. Which parts of
it are read, and when, is covered under [nested configs](#nested-configs).

Unknown keys and unknown tables are ignored on read, except inside
[`[artefact]`](#artefact), [`[resolve]`](#resolve), [`[branches]`](#branches),
[`[topic]`](#topic) and [`[forward]`](#forward), where a misspelt key would
quietly do less than meant. [`git scale require` and `unrequire`](cli.md#git-scale-require--unrequire)
and [`git upgrade`](cli.md#git-upgrade) edit the file in place: comments, key
order and unknown tables are kept.

## A complete example

```toml
[repos]
"imports/core"  = { url = "git@github.com:org/core.git", revision = "main" }
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
"meta/frontend" = { url = "https://github.com/org/frontend.git", revision = "main" }
"vendor/tools"  = { url = "https://github.com/org/tools.git", recursive = false }

[registries]
"gitlab.corp.example" = "registry.corp.example"

[artefact]
include = ["dist/**"]
exclude = ["**/*.map"]

[branches]
pinned = ["main", "staging", "release/*"]

[topic]
prefix = "{user}/"

[clean]
exclude     = [".vscode", ".idea", ".env", "envs/", "tmp"]
keep_recent = "3months"

[hooks]
post_sync     = "make install"
on_pull_error = "warn"

[forward]
parallel = 8
```

Every table is optional. A config with nothing but `[clean]` in it is valid and
useful — see [per-repo `[clean]`](clean.md#per-repo-clean).

## `[repos]`

A table of entries, keyed by the directory the checkout lands in, relative to
the config.

```toml
[repos]
"imports/core" = { url = "git@github.com:org/core.git", revision = "main", recursive = true }
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `url` | string | **required** | Repository URL: HTTPS, SSH (`git@host:owner/repo.git` or `ssh://git@host/owner/repo.git`), or a local path |
| `revision` | string | `""` | Branch, tag or full commit SHA (40 or 64 hex digits). A minimum: [resolution](recursive-dependencies.md#how-a-revision-is-chosen) may raise it to what a dependency asks for. Empty asks for nothing, leaving it to the dependencies — or, when nobody asks, the remote's default branch. See [pinning a revision](dependencies.md#pinning-a-revision) |
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

## `[artefact]`

What [`gitscale artefact publish`](artefacts.md#artefact-publish) ships from
this repository: its build output, as one image layer per group of glob
patterns.

```toml
[[artefact.layer]]
name    = "vendor"
include = ["dist/vendor/**"]

[[artefact.layer]]
name    = "app"
include = ["dist/**"]
exclude = ["**/*.map"]
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `include` | array of strings | — | For a single group: the files to ship |
| `exclude` | array of strings | `[]` | For a single group: files to leave out of what `include` matched |
| `layer` | array of tables | — | One table per group, in layer order, each with `name`, `include` and an optional `exclude`. Use either these or `include` directly, not both |

A layer's `name` is letters, digits, `.`, `_` or `-`, unique, and recorded as
the layer's title. Patterns are [plain globs](artefacts.md#patterns) relative to
the repository, and a file is stored at its repository path; one that starts with `/`, contains `.` or `..` components, or does not
compile is an error when the config is read. A file goes to the first group
that matches it, and a group that matches nothing fails the publish. Every
file shipped must be tracked and unmodified at the commit, or ignored — the
[artefact policy](artefacts.md#the-artefact-policy).

This table belongs to the producing repository. A workspace that takes it as
an artefact never reads it: what it ships is in the image. How a checkout
arrives is no config's to say — see [`git scale prefer`](artefacts.md#choosing-how-a-checkout-arrives).

## `[branches]`

This repository's long-lived branches, its **pinned** ones. A pinned branch is
built from pins, never a [topic](topics.md); merges into it are
[gated](topics.md#topics-in-ci); and the version tags it holds are the
repository's releases.

```toml
[branches]
pinned = ["main", "staging", "release/*"]
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `pinned` | array of strings | the remote's default branch (`main` and `master` when unknown) | Branch names or globs (`*` matches any run of characters). A written list is exactly what is pinned: the default branch is not implied, and `[]` makes every branch a topic |

Where it is read:

- **In the root:** whether the root's branch is a topic, and whether
  [`git scale check`](cli.md#git-scale-check) gates a merge into it.
- **In a dependency, at its default branch:** which tags are its releases, for
  [`git upgrade`](topics.md#promotion-git-upgrade). A pin older than the list
  follows it too.
- **In a dependency, at the revision selected:** when the topic's branch is one
  it pins, what it asks for stays at its pins — see
  [pinned dependencies](topics.md#inside-a-topic).

## `[topic]`

How `git topic start` names topic branches. Read from the root only.

```toml
[topic]
prefix = "{user}/"
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `prefix` | string | unset | Put in front of every topic branch `git topic start` creates. `{user}` is your name for branches: git config [`gitscale.user`](#git-config-keys), else `$USER`. Unset, nothing is added or stripped |

See [branch prefix](topics.md#branch-prefix).

## `[clean]`

Which untracked files [`git scale clean`](clean.md) keeps in **this** repository.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `exclude` | array of strings | `[]` | `.gitignore`-syntax patterns, anchored at this repository's root, passed to `git clean -e` unchanged |
| `keep_recent` | string | `"3months"` | Root only: how recently an image in the root's [image store](stores.md#images) must have been used to survive the daily prune and [`git scale gc`](clean.md#compacting-git-scale-gc). A number and a unit: `30d`, `2 weeks`, `6months` |

`exclude` is scoped to the repository whose config it appears in, and nothing
below it. A pattern may not be empty or start with `-`.

## `[hooks]`

Commands GitScale runs after its own operations. Not to be confused with
[git hooks](hooks.md#git-hooks), which is how GitScale hooks *into* git.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `post_sync` | string | — | Shell command run at the end of every [placement](workflow.md#placement) that succeeded, via `sh -c` in the workspace root. A non-zero exit fails the command |
| `on_pull_error` | string | `"fail"` under CI, `"warn"` otherwise | What a placement a **git hook** ran does when it fails: `"fail"` returns non-zero and fails the git operation, `"warn"` reports and lets it succeed |

Under an installed [git hook](hooks.md#git-hooks), `post_sync` runs only for
workspaces on that hook's [allowlist](hooks.md#the-hook-allowlist).

## `[forward]`

How [git commands run across the workspace](cli.md#git-commands-git-scale-git-command)
are run. Read from the root only.

```toml
[forward]
parallel = 8
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `parallel` | integer, at least 1 | unset | Run `git scale fetch`, `pull` and `push` — after alias lookup — in this many repositories at once. Other git commands run in sequence unless given `--parallel` |

`--parallel=N` on the command line overrides it; `--parallel=1` runs in
sequence.

## Nested configs

When a checked-out repository carries its own `.gitscale.toml` and the entry is
`recursive = true` (the default), GitScale reads these from it:

- **`[repos]`** — to resolve [transitive dependencies](recursive-dependencies.md):
  every entry is a request, resolved with the rest of the workspace, checked
  out once and linked instead of nested.
- **`singleton`** — whether the repository allows only one checkout of itself.
- **`[branches]`** — which branches it [pins](#branches): for its
  dependencies at the revision selected, and for its releases, read by
  `upgrade` from its default branch.
- **`[clean]`** — to decide what [`clean`](clean.md) keeps in that repository.

Everything else in a nested config — `[resolve]`, `[registries]`, `[artefact]`,
`[topic]`, `[hooks]`, `[forward]` — is not even parsed by the parent, so
nothing in those tables can break it. It belongs to that repository when it is
cloned as a workspace root of its own.

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
| A `clean.exclude` pattern that is empty or starts with `-` | It reaches `git clean -e` as an argument |

These checks are independent of the [hook allowlist](hooks.md#the-hook-allowlist),
which covers the `[hooks]` table, and apply even to a placement you started
yourself.

## Git config keys

Two settings are one person's preference rather than the workspace's, so they
are git config, not `.gitscale.toml`:

| Key | Meaning |
|---|---|
| `gitscale.user` | The name `{user}` stands for in a [`[topic] prefix`](#topic). Unset: `$USER` (`USERNAME` on Windows) |
| `gitscale.topic.worktree` | `true` or `false`: whether `git topic start` and `switch` use a worktree of their own by default. Unset: a worktree when the root is a bare repository — see [worktree layout](topics.md#worktree-layout) |

```
git config --global gitscale.user andrey
git config --global gitscale.topic.worktree true
```

## Environment variables

### Read by GitScale

| Variable | Effect |
|---|---|
| `CI` | `1` or `true` switches to [CI behaviour](stores.md#the-ci-cache): depth-1 checkouts from the per-user cache, resolution [always against the remotes](recursive-dependencies.md#when-resolution-asks-the-remotes), and [`git scale clean -fdx` of every checkout placed](hooks.md#git-hooks-in-ci) |
| `GITSCALE_CACHE_DIR` | The CI cache's location |
| `XDG_DATA_HOME` | `$XDG_DATA_HOME/gitscale` is the CI cache's location, when `GITSCALE_CACHE_DIR` is not set |
| `HOME` | `~/.local/share/gitscale` is the last fallback; also where `--global` hooks are installed |
| `NO_COLOR`, `TERM` | `NO_COLOR` set to anything, or `TERM=dumb`, turns colour and icons off — see [colour](cli.md#colour) |
| `USER`, `USERNAME` | The name `{user}` stands for in a [`[topic] prefix`](#topic) when `gitscale.user` is not set |
| `GITSCALE_NO_CI_AUTH` | Any non-empty value disables [CI authentication](ci-authentication.md) |
| `GITSCALE_HOOK_ALLOW` | Set by an installed [git hook shim](hooks.md#the-hook-allowlist) to the allowlist it was installed with. Not something to set yourself |
| `GITSCALE_HOOK` | Set by GitScale on every git call it makes, so an installed hook can tell re-entry from a genuine user operation |
| `CI_MERGE_REQUEST_SOURCE_BRANCH_NAME`, `CI_COMMIT_BRANCH`, `CI_DEFAULT_BRANCH`, `CI_MERGE_REQUEST_TARGET_BRANCH_NAME` | GitLab — the [topic of a pipeline](topics.md#topics-in-ci), the default branch, and the merge request's target for `check` |
| `GITHUB_HEAD_REF`, `GITHUB_REF_NAME`, `GITHUB_REF_TYPE`, `GITHUB_BASE_REF`, `GITHUB_EVENT_PATH` | GitHub — the same |
| `GITLAB_CI` | `true` makes a hook-triggered placement check that the runner's post-checkout clean keeps the declared checkouts, and [fail if it would not](hooks.md#git-hooks-in-ci) |
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

[← 2.11 The agent skill](agents.md) · [Contents](README.md) · [Next → 4. Command line reference](cli.md)

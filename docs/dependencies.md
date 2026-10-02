# 2.1 Declaring dependencies

- [The config file](#the-config-file)
- [Ignoring the checkout directory](#ignoring-the-checkout-directory)
- [Adding and removing entries](#adding-and-removing-entries)
- [What a checkout is](#what-a-checkout-is)
- [Artefacts: `replace` and `overlay`](#artefacts-replace-and-overlay)
- [Pinning a revision](#pinning-a-revision)
  - [Branch](#branch)
  - [Tag](#tag)
  - [Commit SHA](#commit-sha)
  - [No revision](#no-revision)
- [Revision kinds](#revision-kinds)
- [Holding a dependency down: `override`](#holding-a-dependency-down-override)
- [One checkout only: `singleton`](#one-checkout-only-singleton)
- [History and depth](#history-and-depth)
- [Recursive dependencies](#recursive-dependencies)

## The config file

`.gitscale.toml` lives at the top of the workspace's root repository. Every
command searches upward from the current directory until it finds one, so
commands work from anywhere inside the workspace. `-C, --root PATH` overrides
where the search starts.

```toml
[repos]
"imports/core"  = { url = "git@github.com:org/core.git", revision = "main" }
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
"meta/frontend" = { url = "https://github.com/org/frontend.git", revision = "main", artefact = "replace" }
"vendor/tools"  = { url = "https://github.com/org/tools.git", revision = "main", recursive = false }
```

The key is the directory the checkout lands in, relative to the config. Each
entry takes:

| Key | Required | Default | Meaning |
|---|---|---|---|
| `url` | yes | — | Git repository URL: HTTPS, SSH (`git@host:owner/repo.git` or `ssh://…`), or a local path |
| `revision` | no | the remote's default branch | Branch, tag or commit SHA — see [pinning a revision](#pinning-a-revision) |
| `artefact` | no | — | `replace` or `overlay` — see [artefacts](#artefacts-replace-and-overlay) |
| `recursive` | no | `true` | Whether to read this repo's own `.gitscale.toml` — see [recursive dependencies](recursive-dependencies.md) |

Directories must be relative and free of `..`; URLs and revisions may not start
with `-` or use a `helper::` remote-helper prefix. The reasoning, and the full
list of values GitScale refuses, is in
[configuration → values refused](configuration.md#values-gitscale-refuses-to-pass-to-git).

## Ignoring the checkout directory

Keep every declared checkout under one directory — `imports/` throughout this
documentation — and **add it to the workspace repository's `.gitignore`**:

```gitignore
/imports/
```

The checkouts are not content of the workspace repository. What that repository
tracks is the *selection*: `.gitscale.toml`, which names each repository and the
revision it is pinned to. The checkouts themselves are reproduced from it by
`gitscale pull`.

Without the ignore rule, every checkout is an untracked directory in the
workspace repository, which has three visible consequences:

- `git status` in the workspace repository is permanently noisy, listing
  directories GitScale put there. The same happens one level down, where it also
  shows up in [`gitscale status`](status.md): a sub-repository that declares its
  own dependencies reads as [`dirty`](status.md#status-flags) purely because of
  the symlinks planted in it.
- [`gitscale commit`](workflow.md#commit) with no names runs `git add -A` in the
  workspace repository. A git checkout would be staged as an embedded
  repository; an [artefact](#artefacts-replace-and-overlay) checkout, which has no `.git` directory
  at all, would have its entire extracted contents committed.
- Anyone reading a diff has to scroll past it.

The same applies one level down. A repository that declares its own dependencies
gets [symlinks](recursive-dependencies.md) planted at those paths in its working
tree, and those are untracked files in *that* repository — so it should ignore
its own import directory in its own `.gitignore`.

Ignoring is safe with [`gitscale clean`](clean.md), which removes ignored files
(`git clean -xd`) but always excludes the declared checkouts and GitScale's own
symlinks, at every level. It is **not** safe with a hand-run `git clean -xdf` at
the workspace root, which has no such knowledge and will delete the whole
workspace.

Nothing enforces the `imports/` name or requires the checkouts to share a
directory — an entry may be declared at any relative path. One ignored directory
is simply the arrangement that keeps the workspace repository clean with a
single line.

## Adding and removing entries

```
gitscale add imports/core https://github.com/org/core.git main
gitscale add meta/svc     https://github.com/org/svc.git  main --artefact replace

gitscale remove imports/core
```

`add` takes `DIRECTORY REPO_URL REVISION`, all three required, plus an optional
`--artefact replace|overlay`. It refuses a directory that is already declared.
If no config exists yet, it creates one next to `--root` (or the current
directory).

Neither command touches the filesystem — they edit `.gitscale.toml` only. Run
[`gitscale pull`](workflow.md#pull) afterwards to materialise a new entry,
and [`gitscale sync`](workflow.md#sync) after `gitscale remove` to remove its
checkout — which it does only when that loses nothing (files git ignores, such
as build output, go with it).
[`clean`](clean.md) deliberately never deletes checkouts.

> **Both commands rewrite the whole file.** The config is re-emitted from what
> GitScale parsed, so comments, key order and formatting are lost, and any table
> GitScale does not know about is dropped. Edit `.gitscale.toml` by hand if you
> keep comments in it. Entries come back sorted by directory, and keep
> `recursive = false`, `override` and `singleton` when the file said so.

## What a checkout is

Every git entry is checked out as a worktree of one bare clone of its
repository, kept in the root's own `.git` — see [stores](stores.md). It is
**detached at the commit its revision resolves to, and read-only**: every file
in the working tree has its write bits cleared, so an accidental edit fails
immediately instead of producing a change that later gets lost.

To change a repository, put it on the workspace's topic — the root's branch —
with [`gitscale develop`](topics.md#gitscale-develop): it gets a branch of the
topic's name, writable, and `commit` and `push` act on it. Everything else
stays at its pin. See [topics](topics.md).

## Artefacts: `replace` and `overlay`

A repository whose pipeline publishes its build output to an OCI registry —
GitLab's, GHCR, or any other — as one image per commit can be consumed as that
output:

| `artefact` | On disk | Typical use |
|---|---|---|
| `replace` | The image of the commit, read-only, instead of a checkout | Datasets, generated clients, compiled assets nobody builds locally |
| `overlay` | A checkout of the commit, with its build output laid over it | A library whose sources you browse, but whose build you do not want to repeat |

```toml
"meta/frontend" = { url = "https://github.com/org/frontend.git", revision = "main", artefact = "replace" }
"imports/core"  = { url = "https://github.com/org/core.git", revision = "v2.0.0", artefact = "overlay" }
```

The revision resolves exactly as for a git entry, so the files always match
the commit a checkout would get; a commit with no image is an error. The
image's location follows from `url` (`ghcr.io/org/frontend/gitscale` here).

Publishing, registries, logging in, topics and status flags are all on
[their own page](artefacts.md).

## Pinning a revision

`revision` accepts three kinds of value. Whichever it is, the checkout is
detached at one commit; what differs is how that commit is found, and whether
it moves.

### Branch

```toml
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "main" }
```

The checkout is **detached at the branch's tip**, and each `pull` moves it to
the new tip. It never lands on the branch itself: a branch is something a
repository is developed on, and that is what [topics](topics.md) are for.

### Tag

```toml
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
```

Detached at the commit the tag points at. `gitscale status` reports a mismatch
only when HEAD is not at that commit.

### Commit SHA

```toml
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "9fceb02..." }
```

Detached at exactly that commit. On a developer machine it comes from the
store, which has every branch and tag of the remote; in CI, where every
checkout is one commit, it is fetched by its SHA at depth 1. Some git servers
refuse to serve an arbitrary commit; GitHub and GitLab both permit it.

> **A commit is always its full SHA**: 40 hex digits, or 64 in a repository
> using SHA-256 (`git init --object-format=sha256`). Anything else is a branch
> or tag name — so a tag called `20241001` is checked out as the tag it is.
> An abbreviated SHA is refused, on a developer machine as in CI: a full clone
> could expand it, a depth-1 fetch cannot, and an entry must not work in one
> place and fail in the other. The error says to run `git rev-parse <short>` in
> a checkout to get the full one.

### No revision

Omitting `revision` asks for nothing: the repositories that depend on it
decide, through [resolution](recursive-dependencies.md#how-a-revision-is-chosen),
which is how a root config defers the decision to the repositories that
actually care. When nobody asks for a revision, the checkout follows the
remote's default branch, as a [branch](#branch) revision would.

## Revision kinds

What a revision *is* decides how it compares with another request for the same
repository — see [comparing two requests](recursive-dependencies.md#comparing-two-requests).
Only a tag is ever read as a version; a branch called `v2.0.0` is a branch.

| Kind | Recognised by | Compared |
|---|---|---|
| Semver | A tag `MAJOR.MINOR.PATCH[-pre][+build]`, with `v` or without: `v1.2.3`, `1.2.3` | By semver precedence, within one major (`0.N` for `0.x`) |
| Calendar version | A tag whose first number is a four-digit year: `v2026.10.01`, `2026.10.01-2` | By date, then modifier |
| Branch, commit, any other tag | — | By position in the graph, never by history |

**Calendar versions** follow [CalVer](https://calver.org/), spelt like semver
tags: `v` or nothing, then `YYYY.0M.0D` (or `YYYY.0M.MICRO`), then an optional
modifier after a hyphen. Within one date:

- a text modifier is a pre-release and comes first: `2026.10.01-rc1` before `2026.10.01`;
- a numeric modifier is a later release that day, compared as a number: `2026.10.01` before `-2` before `-11`;
- text modifiers compare naturally, so `rc2` comes before `rc10`.

```
2026.10.01-dev < 2026.10.01-rc2 < 2026.10.01-rc10 < 2026.10.01
               < 2026.10.01-2   < 2026.10.01-11   < 2026.10.02
```

Semver tools read a numeric `-2` as a pre-release instead, so a repository
whose tags other tools also read should keep to text modifiers. Short-year
tags such as `26.10.0` look exactly like semver and are read as semver; `26.10`,
with only two numbers, is not a version at all and is decided by position.

**Streams.** Text before the version other than a lone `v` names a stream:
`api-v1.4.0` and `api-1.4.1` are both stream `api-`, and versions are only ever
compared within one. A monorepo can tag `api-…` and `web-…` side by side.
`v` and no prefix are the same stream.

**Recommended for producers:** `vYYYY.0M.0D`, or `vMAJOR.MINOR.PATCH`, and no
other prefix unless the repository releases more than one product.

## Holding a dependency down: `override`

A revision is a minimum: resolution may raise it to what a dependency needs.
`override = true` asks for exactly that revision instead, and wins over every
request from a repository below this one — see
[overrides](recursive-dependencies.md#overrides).

```toml
"imports/d" = { url = "git@github.com:org/d.git", revision = "v1.4.0", override = true }
```

## One checkout only: `singleton`

`singleton = true` says this repository may be checked out only once, whatever
majors are asked for; `singleton = false` on the root's entry relaxes a
dependency's `true`. A repository can say it of itself with a top-level
`singleton = true` in its own `.gitscale.toml`. See
[singleton](recursive-dependencies.md#singleton).

## History and depth

On a developer machine every checkout has its repository's whole history: it
is a worktree of a full bare clone, fetched once per `pull` however many
checkouts and root worktrees use it.

In CI (`CI=1` or `CI=true`) every checkout is the one commit it needs, at depth
1, served from the per-user [cache](stores.md#the-ci-cache) when the runner has
it. A shallow checkout cannot report an exact behind count, so `gitscale
status` shows `≠ stale` when its commit differs from upstream.

## Recursive dependencies

If a repository carries its own `.gitscale.toml`, GitScale reads it: what it
declares is resolved with everything else, checked out once — under the
[hoist directory](configuration.md#resolve), `imports/` unless configured, when
the root does not declare it — and linked in place of a second
checkout. Turn it off per entry with `recursive = false`. The whole mechanism
is [its own page](recursive-dependencies.md).

---

[← 1. Overview](overview.md) · [Contents](README.md) · [Next → 2.2 Status](status.md)

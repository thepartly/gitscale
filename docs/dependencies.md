# 2.1 Declaring dependencies

- [The config file](#the-config-file)
- [Ignoring the checkout directory](#ignoring-the-checkout-directory)
- [Adding and removing entries](#adding-and-removing-entries)
- [Checkout modes](#checkout-modes)
  - [readwrite](#readwrite-default)
  - [readonly](#readonly)
  - [artefact](#artefact)
- [Pinning a revision](#pinning-a-revision)
  - [Branch](#branch)
  - [Tag](#tag)
  - [Commit SHA](#commit-sha)
  - [No revision](#no-revision)
- [Shallow clones](#shallow-clones)
- [Recursive dependencies](#recursive-dependencies)

## The config file

`.gitscale.toml` lives at the root of the workspace. Every command searches
upward from the current directory until it finds one, so commands work from
anywhere inside the workspace. `-C, --root PATH` overrides where the search
starts.

```toml
[repos]
"imports/core"  = { url = "git@github.com:org/core.git", revision = "main", mode = "readonly" }
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
"meta/frontend" = { url = "https://github.com/org/frontend.git", revision = "main", mode = "artefact" }
"vendor/tools"  = { url = "https://github.com/org/tools.git", revision = "main", recursive = false }
```

The key is the directory the checkout lands in, relative to the config. Each
entry takes:

| Key | Required | Default | Meaning |
|---|---|---|---|
| `url` | yes | — | Git repository URL: HTTPS, SSH (`git@host:owner/repo.git` or `ssh://…`), or a local path |
| `revision` | no | the remote's default branch | Branch, tag or commit SHA — see [pinning a revision](#pinning-a-revision) |
| `mode` | no | `readwrite` | `readwrite`, `readonly` or `artefact` — see [checkout modes](#checkout-modes) |
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
`gitscale clone`.

Without the ignore rule, every checkout is an untracked directory in the
workspace repository, which has three visible consequences:

- `git status` in the workspace repository is permanently noisy, listing
  directories GitScale put there. The same happens one level down, where it also
  shows up in [`gitscale status`](status.md): a sub-repository that declares its
  own dependencies reads as [`dirty`](status.md#status-flags) purely because of
  the symlinks planted in it.
- [`gitscale commit`](workflow.md#commit) with no names runs `git add -A` in the
  workspace repository. A git checkout would be staged as an embedded
  repository; an [artefact](#artefact) checkout, which has no `.git` directory
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
gitscale add imports/core https://github.com/org/core.git main --mode readonly
gitscale add meta/svc  https://github.com/org/svc.git  main --mode artefact

gitscale remove imports/core
```

`add` takes `DIRECTORY REPO_URL REVISION`, all three required, plus an optional
`--mode`. It refuses a directory that is already declared. If no config exists
yet, it creates one next to `--root` (or the current directory).

Neither command touches the filesystem — they edit `.gitscale.toml` only. Run
[`gitscale clone`](workflow.md#clone) afterwards to materialise a new entry,
and [`gitscale sync`](workflow.md#sync) after `gitscale remove` to remove its
checkout — which it does only when that loses nothing (files git ignores, such
as build output, go with it).
[`clean`](clean.md) deliberately never deletes checkouts.

> **Both commands rewrite the whole file.** The config is re-emitted from what
> GitScale parsed, so comments, key order and formatting are lost, and any table
> GitScale does not know about is dropped. Edit `.gitscale.toml` by hand if you
> keep comments in it. Entries come back sorted by directory, and keep
> `recursive = false`, `override` and `singleton` when the file said so.

## Checkout modes

| | Clone | Files on disk | `push` | Objects |
|---|---|---|---|---|
| `readwrite` | full git clone | writable | pushed | git |
| `readonly` | git clone, write bits stripped | read-only | skipped | git |
| `artefact` | no clone at all | read-only | skipped | an OCI image from a registry |

### readwrite (default)

An ordinary git clone with full read/write access. This is the mode for a
repository you are actually developing in: `pull` fast-forwards it, `push`
pushes it, `commit` commits it.

### readonly

Cloned like any other repository, then every file in the working tree has its
write bits cleared (`.git/` is left alone, and symlinks are skipped). Use it for
vendored dependencies nobody should edit in place: an accidental write fails
immediately instead of producing a change that later gets lost.

`pull` and `sync` restore write permission, update the checkout, then strip it
again. `push` and `commit` skip readonly entries.

Readonly repos are shallow-cloned when GitScale is talking to a remote directly;
with the [object cache](caching.md) on — the default — they are cloned in full
from a local mirror instead, because borrowing from a mirror makes depth
pointless. See [shallow clones](#shallow-clones).

### artefact

No git clone happens. The source repository's pipeline publishes its build
output to an OCI registry — GitLab's, GHCR, or any other — as one image per
commit, and the entry installs the image of the commit its revision names,
read-only. The revision is resolved with `git ls-remote` exactly as for a git
entry, so the files always match the commit a checkout would get; a commit with
no image is an error.

```toml
"meta/frontend" = { url = "https://github.com/org/frontend.git", revision = "main", mode = "artefact" }
```

The image's location follows from `url` (`ghcr.io/org/frontend/gitscale`
here). A commit has to be spelled out in full, as for
[any entry](#commit-sha). `push` and `commit` skip
artefact entries, [`clean`](clean.md) keeps them whole, and their images are
kept in the [object cache](caching.md).

Publishing, registries, logging in, pipelines and status flags are all on
[their own page](artefacts.md).

## Pinning a revision

`revision` accepts three kinds of value, and what ends up checked out differs.

### Branch

```toml
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "main" }
```

The clone lands **on that branch**, tracking `origin/main`. `pull` fast-forwards
it (`--ff-only`); a branch that has diverged is left alone rather than forced.

### Tag

```toml
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
```

Git leaves **HEAD detached** at the commit the tag points at — that is what git
itself does for a tag, and GitScale does not fight it. `gitscale status` knows
this, and does not report a mismatch as long as HEAD is at the right commit.

### Commit SHA

```toml
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "9fceb02..." }
```

`git clone --branch` accepts only branch and tag names, so a SHA cannot be
cloned shallowly the usual way. When a shallow clone is called for, GitScale
builds the repository directly instead:

```sh
git init && git remote add origin <url>
git fetch --depth 1 origin <sha>
git checkout --detach FETCH_HEAD
```

The result is a shallow clone with a detached HEAD at exactly the pinned commit.
This matters most in CI, where everything is shallow — an entry that clones fine
locally would otherwise fail there.

Some git servers refuse to serve an arbitrary commit. If the fetch is rejected,
GitScale falls back to a full clone and checks the revision out normally. GitHub
and GitLab both permit it.

> **A commit is always its full SHA**: 40 hex digits, or 64 in a repository
> using SHA-256 (`git init --object-format=sha256`). Anything else is a branch
> or tag name — so a tag called `20241001` is checked out as the tag it is.
> An abbreviated SHA is refused, on a developer machine as in CI: a full clone
> could expand it, a shallow one cannot, and an entry must not work in one place
> and fail in the other. The error says to run `git rev-parse <short>` in a
> checkout to get the full one.

### No revision

Omitting `revision` asks for nothing: the repositories that depend on it
decide, through [resolution](recursive-dependencies.md#how-a-revision-is-chosen),
which is how a root config defers the decision to the repositories that
actually care. When nobody asks for a revision, a fresh clone lands on the
remote's default branch, and a pull keeps the checkout on the branch it is on.

For an artefact entry, an omitted revision means the commit the remote's
default branch is on.

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

## Shallow clones

Depth depends on where the objects come from, not only on the mode:

| Context | readwrite | readonly | artefact |
|---|---|---|---|
| Local, cache on (default) | full clone | full clone | no git |
| Local, `--no-cache` | full clone | shallow | no git |
| CI (`CI=1` or `CI=true`) | shallow | shallow | no git |

With the [object cache](caching.md) on, a developer machine borrows from a full
mirror, where shallow buys nothing and borrows less cleanly — so a readonly repo
is cloned whole and its history is there to browse at no extra cost. CI is
served by shallow snapshot entries and keeps the depth-1 checkout it has today.

A shallow repo cannot report an exact behind count, so `gitscale status` shows
`≠ stale` when its commit differs from upstream.

## Recursive dependencies

If a repository carries its own `.gitscale.toml`, GitScale reads it: what it
declares is resolved with everything else, checked out once — under the
[hoist directory](configuration.md#resolve), `imports/` unless configured, when
the root does not declare it — and linked in place of a second
checkout. Turn it off per entry with `recursive = false`. The whole mechanism
is [its own page](recursive-dependencies.md).

---

[← 1. Overview](overview.md) · [Contents](README.md) · [Next → 2.2 Status](status.md)

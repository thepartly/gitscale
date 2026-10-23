# 2.1 Declaring dependencies

- [The config file](#the-config-file)
- [Checkouts are kept out of git status](#checkouts-are-kept-out-of-git-status)
- [Adding and removing entries](#adding-and-removing-entries)
- [What a checkout is](#what-a-checkout-is)
- [Artefacts: how a checkout arrives](#artefacts-how-a-checkout-arrives)
- [Pinning a revision](#pinning-a-revision)
  - [Branch](#branch)
  - [Tag](#tag)
  - [Commit SHA](#commit-sha)
  - [No revision](#no-revision)
- [Revision kinds](#revision-kinds)
- [Holding a dependency down: `override`](#holding-a-dependency-down-override)
- [One checkout only: `singleton`](#one-checkout-only-singleton)
- [Only when needed: `lazy`](#only-when-needed-lazy)
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
"meta/frontend" = { url = "https://github.com/org/frontend.git", revision = "main" }
"vendor/tools"  = { url = "https://github.com/org/tools.git", revision = "main", recursive = false }
```

The key is the directory the checkout lands in, relative to the config. Each
entry takes:

| Key | Required | Default | Meaning |
|---|---|---|---|
| `url` | yes | — | Git repository URL: HTTPS, SSH (`git@host:owner/repo.git` or `ssh://…`), or a local path |
| `revision` | no | the remote's default branch | Branch, tag or commit SHA — see [pinning a revision](#pinning-a-revision) |
| `recursive` | no | `true` | Whether to read this repo's own `.gitscale.toml` — see [recursive dependencies](recursive-dependencies.md) |
| `lazy` | no | `false` | Checked out only once named — see [only when needed](#only-when-needed-lazy) |

Directories must be relative and free of `..`; URLs and revisions may not start
with `-` or use a `helper::` remote-helper prefix. The reasoning, and the full
list of values GitScale refuses, is in
[configuration → values refused](configuration.md#values-gitscale-refuses-to-pass-to-git).

## Checkouts are kept out of git status

The checkouts are not content of the workspace repository. What that repository
tracks is the *selection*: `.gitscale.toml`, which names each repository and the
revision it is pinned to. The checkouts themselves are reproduced from it by
[placement](stores.md#placement).

So GitScale keeps them out of git's sight itself: every placement writes the
checkout directories into the workspace repository's `info/exclude` — git's
own per-repository ignore list, never committed — and, in each checkout, the
[dependency links](recursive-dependencies.md#deduplication-by-symlink) it
planted there into that repository's. `git status` stays quiet, and
[`git scale add -A`](cli.md#git-commands-git-scale-git-command) stages neither
checkouts into the root nor links into a checkout. No `.gitignore` entry is
needed.

The lines are in a block of GitScale's own per worktree, marked `# gitscale:`;
lines of your own in the file are left alone. One file serves every worktree
of a repository, so each root worktree, and each checkout of one store, keeps
its own block, and the block of a worktree that is gone is dropped.

A hand-run `git clean -xdf` — or `-X`, now that the checkouts count as ignored
— at the workspace root deletes the whole workspace. [`git scale
clean`](clean.md) keeps every checkout and link, at every level, whatever the
flags.

Nothing enforces the `imports/` name or requires the checkouts to share a
directory — an entry may be declared at any relative path.

## Adding and removing entries

```
git scale require imports/core https://github.com/org/core.git main

git scale unrequire imports/core
```

`require` takes `DIR URL [REVISION]`; with no revision, none is written. `DIR` is a path
from the current directory. It refuses a directory that is already declared,
and creates the root's `.gitscale.toml` when there is none. `unrequire` refuses
a directory that is not declared.

Both edit the root's `.gitscale.toml` in place — comments, key order and tables
GitScale does not know about are kept — and then [place](stores.md#placement)
the workspace, online: `require` checks the new entry out; `unrequire` leaves
its checkout to the relink step, which removes it when that loses nothing
(files git ignores, such as build output, go with it) and reports it
otherwise. Editing `.gitscale.toml` by hand and running `git scale sync` does
the same. [`clean`](clean.md) never deletes checkouts.

## What a checkout is

Every git entry is checked out as a worktree of one bare clone of its
repository, kept in the root's own `.git` — see [stores](stores.md). It is
**detached at the commit its revision resolves to, and read-only**: every file
in the working tree has its write bits cleared, so an accidental edit fails
immediately instead of producing a change that later gets lost.

To change a repository, put it on the workspace's topic — the root's branch —
with [`git topic join`](topics.md#git-topic-join--leave): it gets a branch of
the topic's name, writable, and
[`git scale commit` and `git scale push`](cli.md#git-commands-git-scale-git-command)
act on it. Everything else stays at its pin. See [topics](topics.md).

## Artefacts: how a checkout arrives

A repository whose pipeline publishes its build output to an OCI registry —
GitLab's, GHCR, or any other — can arrive as the output of its release. Which
form a checkout takes is this workspace's choice, per dependency, never a
config's:

| Form | On disk | Typical use |
|---|---|---|
| `source` | A git checkout — the default | What you develop |
| `artefact` | The image of its release, read-only, instead of a checkout; none of its git history | SDKs, datasets, generated clients, compiled assets nobody builds locally |

```sh
git scale prefer --artefact meta/frontend
git scale pull
```

An artefact is only ever a release's: the revision must resolve to a version
tag, and the image that tag names is installed. A branch, a commit, or a
release with no image fails the checkout. On the topic it is its sources. The
image's location follows from `url` (`ghcr.io/org/frontend/gitscale` here). A
repository whose sources cannot be read arrives as its artefact by itself.

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

The checkout is **detached at the branch's tip**, and each
[placement](stores.md#placement) that asks the remotes — `git scale pull`,
`git scale sync` — moves it to the new tip. It never lands on the branch
itself: a branch is something a repository is developed on, and that is what
[topics](topics.md) are for.

### Tag

```toml
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
```

Detached at the commit the tag points at. `git scale ls` reports a mismatch
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

### Build

```toml
"imports/secret-service" = { url = "https://github.com/org/secret-service.git", revision = "hash:3c9f2a7144e0b2d9..." }
```

One build, named by its [source hash](cli.md#git-scale-hash) — 64 hex digits
after `hash:` — as its pipeline published it to the registry: a colleague's
branch build, say, to try a change before it is released. Its registry
records the commit it was built from: with access to the sources, that commit
is checked out; taken as an [artefact](artefacts.md), or without access, the
build itself is installed. It compares as a commit does, by position.

A build pin is for trying a build out, never for shipping:
[`git scale check`](topics.md#topics-in-ci) refuses to let a config holding
one merge. Set it back to a release by hand; `git upgrade` leaves it alone.
The hash comes from `git scale hash` in the workspace that built it, or from
the `Publishing …:<hash>` line of its pipeline.

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
| Calendar version | A tag `v<major>-` and a date: `v1-2026.10.01`, `v1-2026.10.01-2` | By date, then modifier, within one major |
| Branch, commit, build, any other tag | — | By position in the graph, never by history |

Nothing may come before either form: `api-v1.4.0`, `release-1.4.0` and
`v2026.10.01` are tags like any other, decided by position. The `v` is lower
case.

**Calendar versions** follow [CalVer](https://calver.org/) after a major:
`v<major>-` (a number from 1, no leading zero), then `YYYY.0M.0D` (or
`YYYY.0M.MICRO`), then an optional modifier after a hyphen. The major works as
semver's does: one checkout per major, and `upgrade` crosses one only with
`--major`. Within one date:

- a text modifier is a pre-release and comes first: `v1-2026.10.01-rc1` before `v1-2026.10.01`;
- a numeric modifier is a later release that day, compared as a number: `v1-2026.10.01` before `-2` before `-11`;
- text modifiers compare naturally, so `rc2` comes before `rc10`.

```
v1-2026.10.01-dev < v1-2026.10.01-rc2 < v1-2026.10.01-rc10 < v1-2026.10.01
                  < v1-2026.10.01-2   < v1-2026.10.01-11   < v1-2026.10.02
```

Semver tools read a numeric `-2` as a pre-release instead, so a repository
whose tags other tools also read should keep to text modifiers. Short-year
tags such as `26.10.0` look exactly like semver and are read as semver; `26.10`,
with only two numbers, is not a version at all and is decided by position.

A semver version and a calendar version never compare. A repository moving
from one to the other starts the new scheme at its next major, so the two
are separate checkouts.

**Recommended for producers:** `vMAJOR.MINOR.PATCH`, or `v1-YYYY.0M.0D-N`.

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

## Only when needed: `lazy`

`lazy = true` declares a dependency that only some work needs: a repository of
large test fixtures, or — in a workspace that declares every repository a team
has — each of them.

```toml
"imports/fixtures" = { url = "git@github.com:org/fixtures.git", revision = "v3", lazy = true }
```

A lazy dependency is left out of every placement — `sync`, the git hooks, the
placement after `git topic start` and `switch`, CI — until one of these holds:

- **It is named**: `git scale sync imports/fixtures` checks it out at its pin,
  in the form the workspace takes it in, and `git topic join imports/fixtures`
  checks out its sources and puts it on the topic.
- **It is on disk**, under the directory it would be placed in or a major's
  beside it: from then on it is an ordinary checkout, kept at its pin or on
  its topic branch. Deleting its directory makes it lazy again. `git scale
  prefer --lazy` removes it when that loses nothing, and so do `git topic
  leave` and `git topic finish` for one that `join` brought from nothing. A
  CI build directory never counts: what an earlier job left is no reason to
  place it. One named earlier in the same job does — `CI_JOB_ID` on GitLab,
  the run, attempt and job on GitHub — so `git scale sync imports/fixtures`
  in a job's first step holds for its later commands. So a workspace can hold a lazy checkout a pipeline does not, and
  `prefer --eager` places one it leaves out: a lazy dependency is for work
  that does not need it, and anything the build needs is asked for without
  `lazy`.
- **The topic has a branch of it**: the root's store holds a local branch of
  the topic, or a major's `<topic>@v<major>` — joined in another worktree of
  the root, or before a worktree was deleted by hand — or its remote has one,
  which is how CI and a colleague's workspace pick up a change made there.
- **Another repository asks for it** without `lazy`: then it is an ordinary
  dependency, whatever the workspace prefers.

Until then nothing is fetched for it — on a topic, one ref advertisement asks
its remote for the topic's branch, every lazy dependency's side by side — and
`git scale ls` shows it as `○ lazy` rather than `✘ missed`. A remote that
cannot be asked fails the command in CI and in `git scale check`, so the
merge gate never passes over a change it did not see; on a developer machine
it is a warning, and the repository stays out. Offline, and in the placement the git hooks
run, what its store last fetched answers: the hooks never ask a lazy
remote, so a branch made there since is picked up by the next `git scale
sync`. Where two repositories ask for it under different names, it would
be placed under its repository's own name, as an implicit checkout is. A dependency's own lazy entries are left out the same
way, so a consumer does not inherit what it never uses.

`lazy` is the configs' default. A workspace keeps its own answer with `git
scale prefer --lazy` or `--eager`, and `git scale prefer --unset` hands the
choice back to the configs — see
[choosing how a checkout arrives](artefacts.md#choosing-how-a-checkout-arrives).
`git scale require --lazy` writes the flag. `git upgrade imports/fixtures`
raises a lazy dependency that is not placed: a raise needs its requests and
its releases, not a checkout.

## History and depth

On a developer machine every checkout has its repository's whole history: it
is a worktree of a full bare clone, shared by every checkout and root worktree
that uses it, and fetched once each time resolution asks the remotes.

In CI (`CI=1` or `CI=true`) every checkout is the one commit it needs, at depth
1, served from the per-user [cache](stores.md#the-ci-cache) when the runner has
it. A shallow checkout cannot report an exact behind count, so `git scale ls`
shows `≠ stale` when its commit differs from upstream.

## Recursive dependencies

If a repository carries its own `.gitscale.toml`, GitScale reads it: what it
declares is resolved with everything else, checked out once — under the
[hoist directory](configuration.md#resolve), `imports/` unless configured, when
the root does not declare it — and linked in place of a second
checkout. Turn it off per entry with `recursive = false`. The whole mechanism
is [its own page](recursive-dependencies.md).

---

[← 1. Overview](overview.md) · [Contents](README.md) · [Next → 2.2 The workspace](status.md)

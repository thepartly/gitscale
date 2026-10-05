# 2.5 Stores, worktrees and the CI cache

- [The rule](#the-rule)
- [Where everything lives](#where-everything-lives)
- [What a checkout is](#what-a-checkout-is)
- [A root with worktrees](#a-root-with-worktrees)
- [Moving and deleting](#moving-and-deleting)
- [Images](#images)
- [Checkouts GitScale did not make](#checkouts-gitscale-did-not-make)
- [The CI cache](#the-ci-cache)
  - [cache status](#cache-status)
  - [cache update](#cache-update)
  - [cache compact](#cache-compact)

## The rule

On a developer machine, **everything a workspace needs lives in its root's own
git directory**: one bare clone per dependency, every checkout of it a
worktree, and the artefact images its checkouts were installed from. Nothing
lives outside the root, and deleting the root deletes all of it.

In CI, every checkout is a copy of one exact commit, and **the per-user cache
keeps those commits** — and the images artefact entries were installed from —
so the next job on the runner downloads nothing. Nothing in a job borrows from
the cache: deleting it costs a download, never a checkout.

## Where everything lives

The directory is the root's *common* git directory: the one every worktree of
the root shares — `root/.git` for a plain clone, the bare repository for a
[root that is itself a set of worktrees](#a-root-with-worktrees). Below it,
`<common>` stands for that directory.

```
<common>/gitscale/repos/<name>.git    bare clone per dependency URL; every checkout of it is a worktree
<common>/gitscale/images/<name>/      OCI image layout per artefact repository
<common>/gitscale/locks/              one lock per store, held while it is fetched

one set per root worktree, in that worktree's own git directory:
  gitscale/artefacts/                 what was installed into each artefact checkout
  gitscale/checkouts.json             every checkout gitscale made here
  gitscale/resolve/                   refs of repositories with no store (artefacts only)
```

The main worktree's own git directory is `.git` itself; a linked worktree's is
`<common>/worktrees/<id>/`, which git deletes along with the worktree.

`<name>` is a readable slug of `host/owner/repo` plus a short digest of the
canonical URL, so two repositories that differ only where the slug flattens
them still get separate stores. SSH and HTTPS spellings of one repository share
a store.

A store keeps the remote's branches as `refs/remotes/origin/*`, its tags, and
nothing else of the remote's: a fetch never touches the branches checkouts are
developed on, and no merge request's refs come along. It is an ordinary
repository in every other way — git's own automatic gc runs in it, since git
knows every worktree it has.

A `.gitscale.toml` must sit at the top of a git repository; anywhere else,
GitScale refuses.

## What a checkout is

Every git checkout of the workspace is a worktree of its repository's store:

- **Off the topic**, detached at the commit [resolution](recursive-dependencies.md)
  selected — a SHA as written, a tag or branch resolved to its commit — and
  read-only: the write bit is stripped from every file.
- **On the topic**, on the topic branch and writable — see [topics](topics.md).

Each [placement](workflow.md#placement) moves each checkout where resolution
puts it, fetching each store it asks at most once. A move never loses
anything: uncommitted changes, or commits made at a pin that no branch holds,
fail the entry and leave it where it is.

Because a store is shared, so is everything in it: a branch committed in one
checkout of `imports/core` is visible from every other checkout of it, in every
worktree of the root, and one fetch updates them all.

## A root with worktrees

A worktree of the root — `git topic start --worktree`, or `git worktree add`
— gets worktrees of the same stores for its own checkouts: nothing is
downloaded again, and branches developed in one root worktree are visible in
the others. With the [git hook](hooks.md#git-hooks) installed, `git worktree
add` runs the placement itself.

The root can also be a bare repository with every topic a worktree beside it —
the [worktree layout](topics.md#worktree-layout):

```
git clone --bare git@github.com:acme/app.git app/.git && cd app
git topic switch main                   # app/main; stores in app/.git/gitscale
git topic start PROJ-13-retry           # app/PROJ-13-retry, from the same stores
```

`git clone --bare` sets no fetch refspec, so `origin/*` is never updated and
ahead/behind and upstreams are wrong. The first `git topic` command in this
layout sets it and fetches; a bare root that has never run one gets a warning
from [`git scale ls`](status.md) with the command that sets it.

A root worktree's checkouts are made, moved and pruned by GitScale; a worktree
you add to a checkout yourself — an old release of `core` next to the rest, say
— is left alone.

## Moving and deleting

**Deleting a root worktree** — `git worktree remove`, or `rm -rf` — deletes its
checkouts with it, but leaves an entry for each in its store. Git keeps such an
entry's branch checked out, and refuses it to every other worktree, until the
entry is pruned — so placement, `git topic join` and `leave`, `git scale
clean` and `git scale gc` prune the stores before anything else.

**Moving the root** moves its stores with it, and breaks the links between
them and its checkouts. The next placement repairs them: each checkout
GitScale recorded still names its store, and the store is found by that name
in the moved root. With
git 2.48 or later the links are written relative to each other, and moving the
whole root breaks nothing. A moved bare root needs its own worktrees repaired
first — `git worktree repair` — as git requires of any worktree.

## Images

An artefact entry's images are kept in `<common>/gitscale/images/`: shared by
every worktree of the root, one layer stored once however many commits share
it. Checkouts get copies of what they unpack, so nothing borrows from the store
and removing an image never breaks a checkout.

Each use of an image marks it, and **placement drops images nothing has used
for three months**, then every layer no remaining image needs — at most once a
day, so every other placement pays one `stat` for it. `[clean] keep_recent`
changes the period; [`git scale gc`](clean.md#compacting-git-scale-gc) prunes
now, and runs `git gc` in every store.

## Checkouts GitScale did not make

A checkout that is not a worktree of its store — a clone made by an older
GitScale, or by hand — is reported and never touched:

```
  FAIL  imports/core: not a gitscale worktree; move your changes out, delete it and run git scale sync
```

`git scale ls` flags it `foreign`. A store that is missing while nothing uses
it is cloned again by the next placement.

## The CI cache

CI is detected by `CI=1` or `CI=true`, which GitHub Actions, GitLab CI and most
others set. There, placement and `git scale fetch` use the per-user cache,
unless `--no-cache` is given:

```
<cache>/snapshots/<name>.git          one shallow commit per pin, as refs/heads/pin/<sha>
<cache>/images/<name>/                OCI image layouts, as in a root's image store
<cache>/locks/
```

The cache is `GITSCALE_CACHE_DIR` if set, else `$XDG_DATA_HOME/gitscale`, else
`~/.local/share/gitscale`. It is per user; there is no system-wide scope.

| Action | Network |
|---|---|
| First job on the machine | one depth-1 fetch per commit |
| A later job wanting a commit the cache has | none |
| N parallel jobs, cold | one download per commit, not N |
| Job ends, workspace wiped | the cache keeps everything the job fetched |

A checkout is a local clone of its pin — a copy, detached at the commit and
read-only, its `origin` repointed at the real URL. Without the cache it is one
commit fetched at depth 1 from the remote. Refs always come from the remote: a
stale cache changes how many bytes cross the wire, never which commit a job
gets.

The `cache` commands work on the cache anywhere, `CI` set or not.

### cache status

```
$ git scale cache status
cache  /home/runner/.local/share/gitscale  (2 entries, 3.2 MiB)

  REPO           SNAPSHOTS    IMAGES     TOTAL  REVS  LAST USED
  imports/core    26.2 KiB         -  26.2 KiB     2  just now
      c6e8981  just now
      7373937  6 days ago
  meta/app               -   3.1 MiB   3.1 MiB     1  just now
      3f2a9c1  just now
```

One line per repository, named after the directory this workspace declares it
under, or after the entry's own name when it declares none. Every pin and image
is listed with its own age, since those are what `compact` drops one at a time.

### cache update

```
git scale cache update                 # every declared repository
git scale cache update imports/core    # one of them
```

Add the pins and images a CI job of this workspace would take, touching no
checkout: the way to warm a runner image ahead of time.

### cache compact

```
git scale cache compact
git scale cache compact --keep-recent 2weeks
```

Evict whole entries nothing has used within `--keep-recent` (default
`12months`), then the pins and images inside the ones that stay that have gone
cold, then every blob no remaining image needs. Periods are a number and a
unit: `12h`, `30d`, `2 weeks`, `1month`, `1y`.

---

[← 2.4 Everyday workflow](workflow.md) · [Contents](README.md) · [Next → 2.6 Topics](topics.md)

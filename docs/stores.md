# 2.5 Stores, placement and the CI cache

- [The rule](#the-rule)
- [Where everything lives](#where-everything-lives)
- [What a checkout is](#what-a-checkout-is)
- [Placement](#placement)
  - [Forcing a cleanup](#forcing-a-cleanup)
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
keeps those commits** — and the images artefacts were installed from —
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
  gitscale/resolve/                   refs of repositories with no store: in CI, and ones taken as artefacts
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

Each [placement](#placement) moves each checkout where resolution puts it,
fetching each store it asks at most once.

Because a store is shared, so is everything in it: a branch committed in one
checkout of `imports/core` is visible from every other checkout of it, in every
worktree of the root, and one fetch updates them all.

## Placement

**Placement** is what `git scale sync` does, what the [hook](hooks.md#git-hooks)
and CI run, and what `git scale pull` — and any git command that moved a
`HEAD` — ends with: put every checkout where
[resolution](recursive-dependencies.md#how-a-revision-is-chosen) says it goes,
including a revision a dependency starts asking for in this very placement,
and implicit dependencies new to the graph. Anything missing is checked out
first. In order:

1. **Tidy the stores**: repair checkouts the root's move broke, and prune what
   deleted root worktrees left — see [moving and deleting](#moving-and-deleting).
2. **Carry** the topic, when the root's branch was just created from another
   topic — see [a new branch from a topic](topics.md#a-new-branch-from-a-topic).
3. **Resolve** — online for `sync`, `pull`, the hook in the root and anything
   in CI; fetching only what is missing otherwise. See
   [when resolution asks the remotes](recursive-dependencies.md#when-resolution-asks-the-remotes).
4. **Place** each checkout, below.
5. **Relink**: restore [dependency links](recursive-dependencies.md#deduplication-by-symlink)
   replaced by real checkouts, remove checkouts nothing needs any more, and
   remove orphan links.
6. In CI, **clean** each checkout placed with [`git scale clean -fdx`](clean.md).
7. **Prune** images nothing has used lately, at most once a day — see
   [images](#images).
8. Run the [`post_sync` hook](hooks.md#post_sync), when every step succeeded.

| Entry | What happens |
|---|---|
| Missing directory, or an empty one | A worktree of its store, detached at its commit, read-only |
| Off the topic | Detached at the commit its revision resolves to, read-only. A [branch](dependencies.md#branch) revision moves to the new tip |
| On the topic | On the topic branch, writable, fast-forwarded to its upstream — see [where each checkout goes](topics.md#where-each-checkout-goes) |
| Taken as an [artefact](artefacts.md#choosing-how-a-checkout-arrives) | Nothing to do, and nothing asked of the registry, when the release wanted is the one installed. Otherwise the release's image is downloaded — only the layers the store does not hold — then the files are replaced. A revision that is no release, or a release with no image, fails and leaves the installed files alone. A source checkout there is replaced, unless it holds work |
| Directory holding files but no repository | `FAIL … exists but holds no git repository`. Left as it is — [`git scale clean -fd`](clean.md#a-directory-holding-no-repository) removes it, or move it aside, and run again |
| A checkout that is not a worktree of its store | `FAIL … not a gitscale worktree`, left alone — see [checkouts GitScale did not make](#checkouts-gitscale-did-not-make) |
| The child the [hook fired in](hooks.md#the-hook-in-a-child) | `skip (left where git put it)` |
| A link pointing out of the workspace | `skip (symlink)` |
| Any other symlink | Removed and replaced by a checkout |
| Artefact entry, no registry known for its host | `FAIL … no registry is known for …`, naming the [`[registries]`](configuration.md#registries) entry to add |

A checkout is **moved only when nothing can be lost**. Uncommitted changes to
tracked files fail the entry with `not moved: uncommitted changes; commit or
stash, then pull again`; so do commits made at a detached HEAD that no branch,
tag or remote holds, with the command to keep them. Untracked files do not
block a move: git keeps them, and refuses rather than overwrite one. Commits on
a topic branch never block a move: they stay on the branch in the store.

In CI, where checkouts hold nobody's work, the move is forced, and every
checkout placed is then cleaned — tracked files are updated in place, so
unchanged files keep their mtimes and a restored build cache stays valid.

**Relinking** is the step that can refuse. An
[unlinked checkout](recursive-dependencies.md#unlinked-checkouts) with local
work, a checkout nothing needs but with local work, or an orphan link whose
target still resolves, is reported and left in place, and the command exits
non-zero asking for `--force`. Broken orphans are always removed.

**A checkout nothing needs any more** is one whose entry you removed or
renamed in `.gitscale.toml`, or an implicit dependency no repository asks for
now. Placement removes it when that loses nothing — no uncommitted changes,
unpushed commits or stash, while untracked symlinks (the links GitScale
planted) and files git ignores go with it; an artefact never holds any work,
since every placement replaces it whole. GitScale keeps a record of the
checkouts it manages — `gitscale/checkouts.json` in the root worktree's git
directory — and only those are ever removed: a directory GitScale did not make
is never touched.

An entry that fails — a remote that cannot be reached, a move refused, a
release with no image — does not stop the others:
every other checkout is still placed and linked, and then the command exits
non-zero naming how many failed.

### Forcing a cleanup

```sh
# Pull, then also relink checkouts with local work and remove orphans
git scale pull --force-sync

# Not the same: git pull --force in each repo, normal cleanup
git scale pull --force
```

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

Artefact images are kept in `<common>/gitscale/images/`: shared by
every worktree of the root, one layer stored once however many releases share
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

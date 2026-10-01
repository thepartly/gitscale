# 2.5 The object cache

- [2.5 The object cache](#25-the-object-cache)
  - [The rule](#the-rule)
  - [Where objects come from](#where-objects-come-from)
  - [Where the cache lives](#where-the-cache-lives)
  - [Mirrors and snapshots](#mirrors-and-snapshots)
  - [Artefact entries](#artefact-entries)
  - [What changes in CI](#what-changes-in-ci)
  - [Guarantees and limits](#guarantees-and-limits)
  - [Turning it off](#turning-it-off)
  - [Copying instead of borrowing: dissociate](#copying-instead-of-borrowing-dissociate)
  - [Adopting a root repository](#adopting-a-root-repository)
  - [Seeing what the cache is doing](#seeing-what-the-cache-is-doing)
  - [The cache commands](#the-cache-commands)
    - [cache status](#cache-status)
    - [cache update](#cache-update)
    - [cache adopt](#cache-adopt)
    - [cache repair](#cache-repair)
    - [cache compact](#cache-compact)

## The rule

The cache is one bare git repository per remote URL, kept in the invoking user's
own data directory. It is **on by default**.

One rule governs everything: **the cache talks to the remote, the workspace
talks to the cache.** Every clone, fetch and pull updates the cache entry first,
then builds or updates the workspace from it:

```sh
git -C <cache-entry> remote update -p                      # the only network call
git -C <workspace-repo> fetch <cache-entry> \
    '+refs/heads/*:refs/remotes/origin/*' '+refs/tags/*:refs/tags/*'
```

The second step is local and free. Every workspace on the machine shares the
first — that is the whole saving, and why the entry is updated even when the
workspace could have fetched for itself. Nothing flows back from a workspace,
there is no TTL, and nothing depends on anyone running a maintenance command.

## Where objects come from

Three sources, tried in order:

| Source | When it applies |
|---|---|
| **A source workspace** on this machine | This workspace was derived from another one that already has the repository |
| **The cache entry** | Otherwise — and it is refreshed from the remote first |
| **The remote** | The cache is off, or could not serve this repository |

A *source workspace* is one this workspace came from. Git will not notice such a
relationship by itself, so GitScale works it out:

| How the workspace was made | What identifies the source |
|---|---|
| `git worktree add` | the **main** worktree, from `git worktree list` |
| `git clone --reference` | `objects/info/alternates` |
| an ordinary clone | nothing — sub-repositories come from the cache or the remote |

The source is always the *main* worktree, never a sibling: sibling worktrees get
deleted once their branch merges, and an alternate that disappears leaves the
borrowing repository unable to read its own history.

A copy is only borrowed from when its `origin` matches the configured URL —
occupying the same relative path is not enough, since two unrelated workspaces
may both keep something at `imports/core`. Symlinked (deduped) paths and artefact
entries are skipped. Anything unsuitable falls through to the cache.

This matters most with [git hooks](hooks.md#git-hooks) installed:
`git worktree add` fires `post-checkout`, which pulls, which materialises every
declared repository in the new worktree. Even when objects come from a source
workspace, the cache entry is still refreshed, so the *next* workspace that has
to fall back to it finds it current.

## Where the cache lives

Resolved in this order:

| | |
|---|---|
| `[cache] dir` in the config | as written, with a leading `~/` expanded |
| `GITSCALE_CACHE_DIR` | used as-is |
| `XDG_DATA_HOME` | `$XDG_DATA_HOME/gitscale` |
| otherwise | `~/.local/share/gitscale` |

Inside it:

```
mirror/<name>.git              full mirrors (git entries, developer machines)
snapshots/<name>.git           shallow pin holders (git entries, CI)
artefacts/<name>/              one OCI image layout per artefact repository
  oci-layout
  index.json                   the images held, each annotated with its commit
  blobs/sha256/<digest>        manifests and layers
  gitscale-last-used           touched on every hit
  gitscale-pins/<commit>       touched whenever that commit's image is used
locks/<name>.lock              one advisory lock per entry
```

`<name>` is a readable slug of `host/owner/repo` plus a short digest of the
canonical URL, so two repositories that differ only where the slug flattens them
still get separate entries, and nothing can escape the cache directory.
Different transports of one repository — SSH and HTTPS — share an entry. An
artefact entry has the same name without the `.git`, so a repository's entries
line up across kinds.

**Per-user, and only per-user.** There is no system-wide scope and no shared
directory GitScale will create: a mirror several users write through would give
anything that poisons one entry a machine-wide reach. `dir` still points
wherever you say, including somewhere shared by other means — that is an
arrangement you make, not a mode GitScale sets up.

A workspace being cloned by `gitscale clone <url>` has no config to read yet, so
its root repository uses the environment alone.

## Mirrors and snapshots

Developer machines and CI want opposite things, so there are two kinds of entry.
**Which one is used is decided by the environment, not by the revision**: CI
takes snapshots, everything else takes mirrors.

| | Mirror | Snapshot |
|---|---|---|
| Path | `mirror/<name>.git` | `snapshots/<name>.git` |
| Holds | full history, every branch and tag | one shallow commit per pin, as `refs/heads/pin/<sha>` |
| Consumed by | `git clone --reference` — objects are **borrowed** | an ordinary local clone — objects are **copied** |
| Checkout depth | full | depth 1 |
| Deleting it under a running command | breaks borrowers | harmless |

Git refuses a shallow repository as a `--reference`, so copying is not merely
the safer choice for CI — it is the only one. After the local clone, `origin` is
repointed at the real URL and the temporary `pin/<sha>` branch is dropped.

Resolving a pin costs at most one ref advertisement: a SHA needs no network at
all, and a branch or tag costs one `git ls-remote` — no objects. If the head has
not moved, the commit is already in the entry and nothing further transfers.

## What resolution reads

[Resolution](recursive-dependencies.md#when-resolution-runs-and-what-it-fetches)
needs each repository's branches and tags and the `.gitscale.toml` of each
selected commit — never history. On a developer machine it reads them from the
mirror the checkout is about to be built from, updated once per command, so
resolving costs nothing extra over the wire; an implicit dependency the
workspace has never seen gets a mirror entry like any other.

In CI, and with `--no-cache`, it keeps a small store per repository in the
workspace's own git directory (`.git/gitscale/resolve/`): the refs from
`git ls-remote`, and each config from that one commit fetched at depth 1
without other files — in CI with the cache on, read from the snapshot the
checkout is built from. Artefact repositories always use such a store, so an
artefact's source history is never mirrored just to read its refs.

## Artefact entries

[Artefact](artefacts.md) images are cached too, one OCI image layout per
repository, the same on developer machines and in CI. The rule holds: **refs
come from the remote, bytes come from the cache.** A `clone` or `pull` still
resolves the revision with `ls-remote` and asks the registry for the image's
digest; only the blobs are served locally.

1. The manifest is taken from the entry if its digest is there, otherwise
   downloaded into it.
2. Each layer is taken from the entry if it is there, otherwise downloaded into
   it — under the entry's lock, so N cold jobs at once download each blob once.
3. Every blob is checked against its digest when it is read from the entry; one
   that fails is deleted and downloaded again.
4. The checkout gets **copies**: the layers are unpacked into it, never linked.
5. `gitscale-last-used` and the commit's pin marker are touched.

A layer several commits share — a `vendor` group that rarely changes — is stored
once. Nothing borrows from an artefact entry, so deleting one under a running
command, or all of them, never breaks a workspace: it costs a download.

## What changes in CI

CI is detected by `CI=1` or `CI=true`, which GitHub Actions, GitLab CI and most
others set.

| | Developer machine | CI runner |
|---|---|---|
| Entry kind | mirror | snapshot |
| Checkout depth | full | 1 |
| Object transfer | borrowed via `--reference` | copied by a local clone |
| Source workspace | used when there is one | normally none — the runner clones the root itself |
| Untracked files in checkouts after `pull` | kept | removed, as [`gitscale clean -f`](clean.md) removes them |

The cache being on by default is what makes a shell runner with a persistent
home directory pay off:

| Action | Network |
|---|---|
| First job ever on the machine | one full download per repository |
| Every later job | none for sub-repositories |
| A SHA-pinned repository the cache has | none |
| N parallel jobs, cold | one download total, not N |
| Job ends, workspace wiped | the cache keeps everything the job fetched |

That is one user on one machine — the sharing CI needs is between jobs, not
between users. On hosted runners with nothing persistent between jobs, the cache
costs nothing and saves nothing.

## Guarantees and limits

- **A stale cache changes how many bytes cross the wire, never which commit you
  land on.** Refs always come from the remote, or from an entry that has just
  been updated from it.
- **`origin` in the workspace stays the real URL.** Pushes and a hand-run
  `git fetch` go where they always did; only GitScale's own refresh reads from
  the cache.
- **Every failure degrades to the remote.** A cache that is off, an entry that
  will not update, a revision no snapshot can hold — each falls back to talking
  to the remote directly. Run with `-v` to see when that happens.
- **Concurrent commands do not duplicate work.** Each entry has an advisory lock
  held for the duration of an update, so N cold starts at once cost one
  download.
- **Entries never garbage-collect themselves.** `gc.auto = 0` is set on every
  entry at creation, because `repack -d` can delete objects a live borrower
  still needs. Only [`cache compact`](#cache-compact) repacks.
- **A damaged artefact blob is never used.** Every blob is named by its digest
  and checked against it on each read; a mismatch is deleted and downloaded
  again.

## Turning it off

```
gitscale --no-cache pull        # this command only
```

```toml
[cache]
enabled = false                 # this workspace, always
```

Either way every clone and fetch talks to the remote directly, and depth goes
back to being decided by mode and CI alone — see
[shallow clones](dependencies.md#shallow-clones).

`--no-cache` changes what the next command does, not where the objects a
checkout already borrows happen to live: `gitscale status` still reports an
existing link, and `cache repair` still needs the cache to be on to rebuild one.

## Copying instead of borrowing: dissociate

```toml
[cache]
dissociate = true       # applies to objects borrowed from the cache

[share]
dissociate = true       # applies to objects borrowed from a source workspace
```

By default a new clone keeps a pointer to what it borrowed from and does not
copy the objects, which saves disk as well as network. With `dissociate`, the
borrowed objects are copied in and the pointer dropped once the clone is made:
the network saving remains, the disk saving does not, and the result no longer
depends on the source surviving.

Turn it on where the source may be moved, deleted or garbage-collected. `git gc`
in a source repository does not know it has borrowers and can delete objects one
still needs — nothing warns when that happens.

The two settings are independent because the two sources have different risks: a
cache entry is managed by GitScale and never pruned while it has borrowers; a
source workspace is an ordinary repository somebody may run `git gc` in.

## Adopting a root repository

```toml
[cache]
adopt_root = true
```

Git knows nothing about the object cache, so a workspace root created by plain
`git clone` — which is how people normally clone one — holds its own full copy
and shares nothing. With `adopt_root`, the next `gitscale clone` or `pull` seeds
an entry from it — locally, no network — points the repository at that entry,
and repacks away the objects it no longer needs to own.

That next `pull` is usually the one an installed
[git hook](hooks.md#git-hooks) fires at the end of the clone, so with both in
place a plain `git clone` ends up with a fully materialised workspace whose root
is on the cache like everything else. A root created by
[`gitscale clone <url>`](workflow.md#cloning-a-workspace-from-a-url) is linked
from birth and needs none of this.

**Opt-in, and it stays opt-in.** The reclaim step deletes the repository's own
objects and converts something that stood on its own into something that depends
on the cache. It is recoverable with [`cache repair`](#cache-repair), but it is
your call, not a default.

Adoption is skipped when the repository already borrows from something else —
overwriting that pointer and repacking would leave it unable to read objects it
never owned — when it is a shallow clone, which git will not let a mirror take
refs from, and when the workspace root is not itself the top of a git
repository. It never adopts from a linked worktree, whose object store belongs
to the main worktree — see [`cache adopt`](#cache-adopt), which does the same on
demand and says which of these applies. It is not needed in CI, where the runner
clones the root itself and the workspace is discarded at the end.

`git pull` in the root never goes near the cache, so every `gitscale clone` and
`pull` marks the entry a linked root borrows from as used — adopted or made by
`gitscale clone <url>` alike. Without that, [`compact`](#cache-compact) would
evict it once `--keep-recent` passed, however busy the workspace was, and leave
the root unable to read its own history.

Note what it does and does not buy. It reclaims the root's duplicate objects and
gives every later worktree and workspace on the machine something to borrow. It
does not make the *next* `git clone` of that root cheaper — git will not consult
the cache, so only `gitscale clone <url>` saves that download.

## Seeing what the cache is doing

[`gitscale status`](status.md) stays about the workspace and says nothing about
the cache, with one exception: a checkout borrowing from an entry that has been
deleted is flagged `cache-broken`, because it cannot read its own history and
nothing else would tell you.

`gitscale status --format json` carries the detail per repository, in a `cache`
field — `-`, `mirror`, `snapshot`, `workspace`, `copy` or `broken`. The values
are explained in [status → JSON output](status.md#json-output). Two are worth
expanding on:

- **`copy`** is what every checkout made before this machine had a cache looks
  like, and it is not a fault. `--reference` is decided when a repository is
  cloned and nothing re-links it afterwards, so the objects sit in both places.
  The entry is still kept current and still serves every other workspace; only
  this checkout pays for its own copy, and only in disk. Re-cloning it is all it
  takes to borrow instead.
- **`snapshot`** is not a link on disk — a CI job copies what it takes. What is
  reported is that the entry holds the commit this checkout is on, which is what
  decides whether the next job pays for it again.

## The cache commands

Everything else touches the cache implicitly. These are for when that is not
enough.

### cache status

```
gitscale cache status
gitscale cache status -v        # also list every ref a mirror holds
```

```
cache  /home/dev/.local/share/gitscale  (5 entries, 3.2 MiB)

  REPO             MIRROR  SNAPSHOTS  ARTEFACTS     TOTAL  REVS  LAST USED
  imports/core   26.2 KiB          -          -  26.2 KiB     9  2 hours ago
  imports/utils         -   26.2 KiB          -  26.2 KiB     2  just now
      c6e8981  just now
      7373937  6 days ago
  meta/app              -          -    3.1 MiB   3.1 MiB     2  just now
      3f2a9c1  just now
      9fceb02  3 weeks ago
```

One line per **repository**, not per entry — a machine that both develops and
runs jobs has a mirror and a snapshot for the same repository and pays for both.
Rows are named after the directory this workspace declares them under, or after
the cache's own entry name when this workspace does not declare them.

| Column | Meaning |
|---|---|
| `MIRROR` | The mirror entry's size, or `-` if there is none |
| `SNAPSHOTS` | The snapshot entry's size, holding every pinned commit |
| `ARTEFACTS` | The artefact entry's size, holding every cached image |
| `TOTAL` | All together — what this repository costs the cache |
| `REVS` | Revisions held: a mirror's branches and tags, a snapshot's pins, the commits an artefact entry has images for |
| `LAST USED` | When anything last read any of its entries |

Pins and artefact commits are listed under their row with their own age,
because that is what `compact` drops one at a time. A mirror's refs are counted in `REVS` but listed
only under `-v`: a busy repository has hundreds, and that is not a summary.

Like `compact`, this works from anywhere — the cache belongs to the user, not to
a workspace. A config is used when there is one, so `[cache] dir` is honoured.

### cache update

```
gitscale cache update                 # every declared repository
gitscale cache update imports/core       # one of them
```

Refresh entries without touching any checkout — the one command that warms a
repository nobody has pulled yet. On a developer machine it updates mirrors; in
CI it adds a pin for each entry's revision, reporting
`imports/core (pinned at 9fceb02)`, and skips anything it cannot pin
(`nothing to pin`). An artefact entry downloads the image its revision names
now, on either, reporting `meta/app (artefact 3f2a9c1)`. `-v` prints the
cache's total size afterwards.

### cache adopt

```
gitscale cache adopt
```

Put the workspace root on the cache now, the way
[`adopt_root`](#adopting-a-root-repository) does on the next `clone` or `pull`
— without the setting, and without pulling anything else. When it will not, it
says why: the root already borrows from the cache, borrows from somewhere else,
is a shallow clone, or has no remote-tracking branches to seed an entry with.

In a linked worktree it refuses, because the object store a worktree reads
belongs to its main worktree: adopting would relink main and every sibling
along with it. Run it in the main worktree, or pass `--shared` to do it from
the linked one; either way it names every worktree that moved. `adopt_root`
never adopts from a linked worktree — the pull `git worktree add` fires would
otherwise relink main behind your back — and waits for a pull in main instead.

### cache repair

```
gitscale cache repair
```

```
  repair imports/core
  ok     imports/utils
  skip   imports/tools (not borrowing)
Repaired 1 entry.
```

Re-create the entries this workspace borrows from but that are no longer there
— the state `status` flags as `cache-broken`. A workspace whose entry was
deleted cannot read its own history, and git reports nothing until something
tries to read an object. The workspace's own root repository is checked too,
once it has been [adopted](#adopting-a-root-repository).

An artefact entry has no borrowers to mend. Instead every blob it holds is
checked against its digest, and the damaged ones deleted —
`repair meta/app (1 damaged blob removed)` — to be downloaded again the next
time they are wanted.

### cache compact

```
gitscale cache compact
gitscale cache compact --keep-recent 2weeks
```

Repack every entry and evict the ones nothing has used within `--keep-recent`
(default `12months`). Each entry carries a last-used marker touched on every hit,
and each pinned commit carries its own — so a snapshot entry in daily use can
still shed the pins that have gone cold. An artefact entry drops the images of
the commits whose markers have gone cold, then deletes every blob no remaining
image needs — which also clears what a forced re-publish left behind.

A hit is any `clone`, `fetch`, `pull`, `sync` or `cache update` that goes
through the entry, a CI job taking a pin, adoption, and — for the entry a
workspace root borrows from — any `clone`, `pull` or `sync` in that workspace.
Nothing checks who still borrows: a workspace nobody has touched within the
period loses the entries it borrows from, `status` flags its checkouts
`cache-broken`, and [`cache repair`](#cache-repair) downloads them again.

Periods are a number and a unit: `12h`, `30d`, `2 weeks`, `1month`, `1y`. A bare
number is a count of days.

**Mirrors are repacked but never pruned.** `repack -d` is the one operation that
can delete objects a live borrower still needs, and nothing warns when it does.
Snapshots and artefact entries have no borrowers — a job copies what it takes —
so those are pruned properly.

Works from anywhere, with or without a config.

---

[← 2.4 Everyday workflow](workflow.md) · [Contents](README.md) · [Next → 2.6 Hooks](hooks.md)

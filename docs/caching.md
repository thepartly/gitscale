# 2.5 The object cache

- [The rule](#the-rule)
- [Where objects come from](#where-objects-come-from)
- [Where the cache lives](#where-the-cache-lives)
- [Mirrors and snapshots](#mirrors-and-snapshots)
- [What changes in CI](#what-changes-in-ci)
- [Guarantees and limits](#guarantees-and-limits)
- [Turning it off](#turning-it-off)
- [Copying instead of borrowing: dissociate](#copying-instead-of-borrowing-dissociate)
- [Adopting a root repository](#adopting-a-root-repository)
- [Seeing what the cache is doing](#seeing-what-the-cache-is-doing)
- [The cache commands](#the-cache-commands)

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
mirror/<name>.git       full mirrors
snapshots/<name>.git    shallow pin holders
locks/<name>.lock       one advisory lock per entry
```

`<name>` is a readable slug of `host/owner/repo` plus a short digest of the
canonical URL, so two repositories that differ only where the slug flattens them
still get separate entries, and nothing can escape the cache directory.
Different transports of one repository — SSH and HTTPS — share an entry.

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

## What changes in CI

CI is detected by `CI=1` or `CI=true`, which GitHub Actions, GitLab CI and most
others set.

| | Developer machine | CI runner |
|---|---|---|
| Entry kind | mirror | snapshot |
| Checkout depth | full | 1 |
| Object transfer | borrowed via `--reference` | copied by a local clone |
| Source workspace | used when there is one | normally none — the runner clones the root itself |

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
- **Artefact entries are never cached.** An unpacked archive has no object
  store.

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
never owned — and when the workspace root is not itself the top of a git
repository. It is not needed in CI, where the runner clones the root itself and
the workspace is discarded at the end.

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
cache  /home/dev/.local/share/gitscale  (4 entries, 104.8 KiB)

  REPO             MIRROR  SNAPSHOTS     TOTAL  REVS  LAST USED
  imports/core   26.2 KiB          -  26.2 KiB     9  2 hours ago
  imports/utils         -   26.2 KiB  26.2 KiB     2  just now
      c6e8981d205b  just now
      73739376c62a  6 days ago
```

One line per **repository**, not per entry — a machine that both develops and
runs jobs has a mirror and a snapshot for the same repository and pays for both.
Rows are named after the directory this workspace declares them under, or after
the cache's own entry name when this workspace does not declare them.

| Column | Meaning |
|---|---|
| `MIRROR` | The mirror entry's size, or `-` if there is none |
| `SNAPSHOTS` | The snapshot entry's size, holding every pinned commit |
| `TOTAL` | Both together — what this repository costs the cache |
| `REVS` | Revisions held: a mirror's branches and tags, a snapshot's pins |
| `LAST USED` | When anything last read either entry |

Pins are listed under their row with their own age, because that is what
`compact` drops one at a time. A mirror's refs are counted in `REVS` but listed
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
`imports/core (pinned at 9fceb02a)`, and skips anything it cannot pin
(`nothing to pin`). `-v` prints the cache's total size afterwards.

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

### cache compact

```
gitscale cache compact
gitscale cache compact --keep-recent 2weeks
```

Repack every entry and evict the ones nothing has used within `--keep-recent`
(default `1month`). Each entry carries a last-used marker touched on every hit,
and each pinned commit carries its own — so a snapshot entry in daily use can
still shed the pins that have gone cold.

Periods are a number and a unit: `12h`, `30d`, `2 weeks`, `1month`, `1y`. A bare
number is a count of days.

**Mirrors are repacked but never pruned.** `repack -d` is the one operation that
can delete objects a live borrower still needs, and nothing warns when it does.
Snapshots have no borrowers — a job copies what it takes — so those are pruned
properly.

Works from anywhere, with or without a config.

---

[← 2.4 Everyday workflow](workflow.md) · [Contents](README.md) · [Next → 2.6 Hooks](hooks.md)

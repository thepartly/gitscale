# 2.4 Everyday workflow

- [How the multi-repo commands behave](#how-the-multi-repo-commands-behave)
- [clone](#clone)
  - [Cloning a workspace from a URL](#cloning-a-workspace-from-a-url)
- [fetch](#fetch)
- [pull](#pull)
- [push](#push)
- [sync](#sync)
- [commit](#commit)
- [Choosing between them](#choosing-between-them)

## How the multi-repo commands behave

`clone`, `fetch`, `pull`, `push`, `sync`, `commit` and `clean` all share a shape:

- **Selection.** With no arguments they act on every declared entry. Given
  names, they act on those — the names are the directory keys from
  `.gitscale.toml`, exactly as written. An unknown name is an error
  (`Unknown repos: …`) and nothing runs.
- **Parallelism.** On a terminal, repositories are processed concurrently with
  a progress line each. Redirected to a file or a pipe, they run sequentially
  and print plain lines, which is what scripts and CI logs see.
- **Reporting.** One line per repository on stdout — `ok`, `skip` with the
  reason — and failures on stderr as `FAIL <repo>: <error>`.
- **Exit status.** Any failure means a non-zero exit and a summary such as
  `2 repo(s) failed to pull`. Skips are not failures.
- **Location.** The config is found by searching upward from the current
  directory; `-C, --root PATH` starts the search somewhere else.
- **Network.** Everything goes through the [object cache](caching.md) unless
  `--no-cache` is given.
- **What counts as a checkout.** A directory with a `.git` of its own. One that
  exists but holds no repository — left by a failed clone, an interrupted
  delete, an outside cleaner — is treated as not cloned: git run inside it would
  walk up and act on the workspace's own repository, so GitScale never runs git
  there.

```
Cloning missing repos...
  ok    imports/rw-lib
  skip  imports/ro-lib (already exists)
  ok    meta/art (artefact 3f2a9c1)
```

## clone

Create the checkouts that do not exist yet. Existing checkouts are left
untouched.

```
gitscale clone                  # everything declared
gitscale clone imports/core        # one entry
```

Per entry:

| Situation | What happens |
|---|---|
| Directory missing, git entry | Cloned, through the cache when it can serve it |
| Directory missing, artefact entry | The image of the commit the revision names is downloaded, checked and unpacked read-only — see [artefacts](artefacts.md#the-checkout) |
| Checkout exists | `skip (already exists)` |
| Directory exists but is empty | Cloned into it |
| Directory exists, holds files but no repository | `FAIL … exists but holds no git repository`. Left as it is — [`gitscale clean -f`](clean.md#a-directory-holding-no-repository) removes it, or move it aside, and run again |
| Path is a symlink planted by an enclosing workspace (running inside a child repository) | `skip (symlink)`, unless the entry is named — then unlinked as below |
| Any other symlink | The symlink is removed and a real clone is made in its place |
| Artefact entry, no registry known for its host | `FAIL … no registry is known for …`, naming the [`[registries]`](configuration.md#registries) entry to add |
| Artefact entry, the commit has no image | `FAIL … no artefact for <image>:<commit> (<revision>); its pipeline may not have published yet` |

Afterwards GitScale resolves [recursive dependencies](recursive-dependencies.md):
it reads each checkout's own `.gitscale.toml`, moves any checkout whose revision
is adopted from a child to that revision the way [`pull`](#pull) would (a readonly
checkout stays read-only, a shallow one fetches just that ref; an artefact keeps
what it installed), and plants the dedup symlinks.

Note the symlink rule: a path that is a symlink is replaced by a real clone.
That is how a deduped [recursive dependency](recursive-dependencies.md) is
turned back into its own checkout — but the symlink sits at a path declared by
the *child* config, so the command to run is `gitscale clone imports/shared` from
inside that child repository, not from the workspace root. It has to be named:
run inside a child without names, `clone`, `pull` and `sync` leave the enclosing
workspace's links in place, since that workspace's root decides which revision
those checkouts are at. `sync` at the root does the opposite and relinks it.

### Cloning a workspace from a URL

**The normal way to set up a workspace is `git clone`.** With a `--global` or
`--system` [git hook](hooks.md#git-hooks) installed, the `post-checkout` that
git fires at the end of the clone runs `gitscale pull`, which materialises every
declared repository — and, with
[`[cache] adopt_root`](caching.md#adopting-a-root-repository) set, relinks the
root repository to the object cache at the same time. Nobody has to know
GitScale is involved:

```
git clone https://github.com/org/root.git
```

`gitscale clone` accepts a URL for the cases where that does not apply — no hook
installed, a hook whose [allowlist](hooks.md#the-hook-allowlist) does not cover
the repository, or a one-off checkout on a machine you do not administer. It
clones the root repository, then everything its `.gitscale.toml` declares:

```
gitscale clone https://github.com/org/root.git         # into ./root
gitscale clone git@github.com:org/root.git my-ws       # into ./my-ws
```

The first argument is read as a URL when it has a scheme (`https://`, `file://`),
is an scp-style SSH address (`git@host:owner/repo.git`), or is an absolute path.
Declared directories are always relative and free of `..`, so the two cannot be
confused. A *relative* local path is the one shape that would be ambiguous —
spell it `file://…` or absolutely. At most one further argument is accepted, the
directory to create; without it, git's own rule applies and the repository name
is used.

One thing it does that `git clone` cannot: the root repository goes through the
[object cache](caching.md) like everything else, so it is linked to the cache
from birth and a second workspace of it costs nothing over the wire. Git knows
nothing about the cache, so a plain `git clone` always pays the root's full
download and then joins the cache after the fact, if `adopt_root` says to.

There is no config to read yet at that point, so only the environment and
`--no-cache` decide whether the cache is used; the `[cache]` table inside the
cloned repository governs every step after that.

If the cloned repository has no `.gitscale.toml`, GitScale says so and stops.

## fetch

Update remote state without touching any working tree.

```
gitscale fetch                  # everything
gitscale fetch imports/core        # one entry
```

- **Git entries**: the cache entry is refreshed from the remote, then the
  workspace's remote-tracking refs are updated from it. A shallow checkout stays
  shallow and fetches only the revision its entry pins, so a tag or branch the
  config has moved to arrives too. A checkout that does not exist, or a directory
  holding no repository, is skipped (`not cloned`); a symlinked (deduped) entry is
  skipped (`symlink`) — the real checkout is fetched under its own name.
- **Artefact entries**: the revision is resolved to a commit with `git
  ls-remote` and the registry is asked whether that commit has an image; both
  are recorded, outside the checkout, for [`status`](artefacts.md#status).
  Nothing is downloaded or extracted. A commit with no image fails the entry.

Nothing that `fetch` does can change which commit is checked out — it is safe to
run in a dirty workspace.

## pull

Bring every checkout to where a fresh `clone` would put it: anything missing is
cloned first, and an entry with no revision of its own follows the one a child
config pins, as [`clone`](#clone) does — including a pin the pull itself brings
in.

```
gitscale pull                   # everything
gitscale pull imports/core         # one entry
```

| Entry | What happens |
|---|---|
| Missing directory, or an empty one | Cloned, exactly as `clone` would |
| Directory holding files but no repository | `FAIL`, left as it is, exactly as `clone` would |
| Full clone, on a branch | Refs refreshed, checked out if it is on the wrong revision, then fast-forwarded (`--ff-only`). A diverged branch is left alone rather than forced; a remote that cannot be reached, or local changes that block the fast-forward, fail the entry |
| Full clone, no revision | Stays on the branch it is on, which is fast-forwarded |
| Shallow clone | Only the pinned branch or tag is fetched, at depth 1, and checked out: on the branch, or detached at the tag. With no revision, refetched and reset to the upstream commit |
| Shallow clone pinned to a SHA | That one commit is fetched and reset to |
| CI, served by a cache snapshot | The pinned commit is taken from local disk; no network at all |
| readonly | Made writable, updated, then made read-only again |
| artefact | Nothing to do, and nothing asked of the registry, when the revision still names the installed commit. Otherwise the new image is downloaded — only the layers the cache does not hold — then the files are replaced. A commit with no image fails and leaves the installed files alone |
| Symlink planted by an enclosing workspace (running inside a child repository) | `skip (symlink)` — that checkout and its revision belong to the outer workspace's root. Named, it is unlinked as `clone` would |
| Any other symlink | Removed and replaced by a real clone, as `clone` does |

Afterwards the [recursive dependency](recursive-dependencies.md) symlinks are
re-created — a child config may have changed — and the
[`post_sync` hook](hooks.md#post_sync) runs if one is configured.

This is also the command an [installed git hook](hooks.md#git-hooks) runs, which
is what makes a fresh clone or a new worktree populate itself.

## push

```
gitscale push                   # everything
gitscale push imports/core         # one entry
```

`git push` in each readwrite checkout. `readonly` entries, `artefact` entries,
directories that hold no checkout and symlinked (deduped) entries are skipped. Inside CI, the remote is
repointed at the job-token HTTPS URL first where that applies — see
[CI authentication](ci-authentication.md).

## sync

The one-command "make the workspace match the config", in this order:

1. **clone** everything missing.
2. **Reconcile remotes** — each existing clone's `origin` is set to the
   configured URL, so switching an entry from HTTPS to SSH (or the reverse) in
   `.gitscale.toml` takes effect without re-cloning. Changes are printed as
   `update <repo> -> <url>`.
3. **pull** everything.
4. **Relink** — restore [recursive dependency](recursive-dependencies.md)
   symlinks that have been replaced by real clones, and remove orphaned links.
5. **push** everything pushable.
6. Run the [`post_sync` hook](hooks.md#post_sync).

```
gitscale sync                   # everything
gitscale sync imports/core         # one entry
gitscale sync --force           # also relink modified clones, remove valid-target orphans
```

Step 4 happens *before* the push so that local hygiene is not blocked by a
remote or authentication failure. It is also the step that can refuse: an
unlinked clone with local work, or an orphaned link whose target still resolves,
is reported and left in place, and `sync` exits non-zero asking for `--force`.
Broken orphans are always removed.

## commit

Commit across the workspace with one shared message.

```
gitscale commit -m "bump shared protocol"
gitscale commit -m "wip" imports/core apps/web
```

In each selected checkout this is `git add -A` followed by `git commit -m`, so
untracked files are included. Skipped: `artefact` entries, `readonly` entries,
directories that hold no checkout, symlinked (deduped) entries — the real checkout
is committed under its own name — and any repo whose working tree is already
clean.

When run with no names, and when the workspace root is itself the top level of a
git repository, the workspace repository is committed too, reported as `.`. It
is never committed when specific names were given, and never when the config
root merely sits inside some ancestor repository.

Nothing is pushed. Follow with `gitscale push` or `gitscale sync`.

Because the workspace commit is a `git add -A`, the checkout directory should be
in that repository's `.gitignore` — see
[ignoring the checkout directory](dependencies.md#ignoring-the-checkout-directory).

## Choosing between them

| You want to | Use |
|---|---|
| Set up a workspace | `git clone <url>`, with a [git hook](hooks.md#git-hooks) installed |
| Set up a workspace with no hook installed | `gitscale clone <url>` |
| Materialise entries added to the config | `gitscale clone` |
| See what changed upstream, safely | `gitscale fetch` then `gitscale status` |
| Get up to date | `gitscale pull` |
| Get up to date, fix links, and push | `gitscale sync` |
| Turn a deduped symlink into a real checkout | `gitscale clone <name>` from inside the repository that declares it |
| Turn a real checkout back into a deduped symlink | `gitscale sync` (`--force` if it has local work) |

---

[← 2.3 Recursive dependencies](recursive-dependencies.md) · [Contents](README.md) · [Next → 2.5 The object cache](caching.md)

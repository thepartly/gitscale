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

```
Cloning missing repos...
  ok    imports/rw-lib
  skip  imports/ro-lib (already exists)
  ok    meta/art (artefact)
```

## clone

Create the checkouts that do not exist yet. Existing directories are left
untouched.

```
gitscale clone                  # everything declared
gitscale clone imports/core        # one entry
```

Per entry:

| Situation | What happens |
|---|---|
| Directory missing, git entry | Cloned, through the cache when it can serve it |
| Directory missing, artefact entry | Archive downloaded and extracted |
| Directory exists | `skip (already exists)` |
| Path is a symlink | The symlink is removed and a real clone is made in its place |
| Artefact entry, no `[storage]` url | `FAIL … no [storage] configured` |
| Artefact entry, nothing in storage | `skip (no artefact data)` |

Afterwards GitScale resolves [recursive dependencies](recursive-dependencies.md):
it reads each checkout's own `.gitscale.toml`, checks out any revision adopted
from a child, and plants the dedup symlinks.

Note the symlink rule: a path that is a symlink is replaced by a real clone.
That is how a deduped [recursive dependency](recursive-dependencies.md) is
turned back into its own checkout — but the symlink sits at a path declared by
the *child* config, so the command to run is `gitscale clone imports/shared` from
inside that child repository, not from the workspace root. `sync` at the root
does the opposite and relinks it.

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
  shallow. A checkout that does not exist is skipped (`not cloned`).
- **Artefact entries**: a HEAD request against object storage, recording the
  remote ETag in `.etag-remote`. Nothing is downloaded or extracted.

Nothing that `fetch` does can change which commit is checked out — it is safe to
run in a dirty workspace.

## pull

Bring every checkout up to date, cloning anything that is missing first.

```
gitscale pull                   # everything
gitscale pull imports/core         # one entry
```

| Entry | What happens |
|---|---|
| Missing directory | Cloned, exactly as `clone` would |
| Full clone, on a branch | Refs refreshed, checked out if it is on the wrong revision, then fast-forwarded (`--ff-only`). A diverged branch is left alone rather than forced |
| Shallow clone | Refetched at depth 1 and `reset --hard` to the upstream commit |
| Shallow clone pinned to a SHA | That one commit is fetched and reset to |
| CI, served by a cache snapshot | The pinned commit is taken from local disk; no network at all |
| readonly | Made writable, updated, then made read-only again |
| artefact | ETag compared; re-downloaded and re-extracted only if the remote differs |

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

`git push` in each readwrite checkout. `readonly` entries, `artefact` entries
and directories that do not exist are skipped. Inside CI, the remote is
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
directories that do not exist, symlinked (deduped) entries — the real checkout
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

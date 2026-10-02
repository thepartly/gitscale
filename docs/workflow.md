# 2.4 Everyday workflow

- [How the multi-repo commands behave](#how-the-multi-repo-commands-behave)
- [Getting a workspace](#getting-a-workspace)
- [fetch](#fetch)
- [pull](#pull)
- [push](#push)
- [sync](#sync)
- [commit](#commit)
- [Choosing between them](#choosing-between-them)

## How the multi-repo commands behave

`fetch`, `pull`, `push`, `sync`, `commit` and `clean` all share a shape:

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
- **Where checkouts come from.** On a developer machine, from the root's
  [stores](stores.md); in CI, from the per-user [cache](stores.md#the-ci-cache)
  unless `--no-cache` is given.
- **What counts as a checkout.** A directory with a `.git` of its own. One that
  exists but holds no repository — left by a failed checkout, an interrupted
  delete, an outside cleaner — is treated as not checked out: git run inside it
  would walk up and act on the workspace's own repository, so GitScale never
  runs git there.

```
Pulling latest changes...
  ok    imports/b
  ok    imports/d (on feat/price-cache)
  ok    meta/art (artefact 3f2a9c1)
```

## Getting a workspace

**The normal way to set up a workspace is `git clone`.** With a `--global` or
`--system` [git hook](hooks.md#git-hooks) installed, the `post-checkout` that
git fires at the end of the clone runs `gitscale pull`, which materialises every
declared repository. Nobody has to know GitScale is involved:

```
git clone https://github.com/org/root.git
```

Without a hook, run `gitscale pull` in the new clone. A
[root made of worktrees](stores.md#a-root-with-worktrees) works the same way:
each root worktree gets its own checkouts, all from the same stores.

## fetch

Update remote state without touching any working tree.

```
gitscale fetch                  # everything
gitscale fetch imports/core     # one entry
```

- **Git entries**: the repository's store is fetched — every branch and tag,
  pruned — whether or not it has a checkout yet. A symlinked (deduped) entry is
  skipped (`symlink`): the real checkout is fetched under its own name. In CI
  there is no history to fetch into, so git entries are skipped (`no history in
  CI`); the next `pull` takes the commit it needs.
- **Artefact entries**: the revision is resolved to a commit with `git
  ls-remote` and the registry is asked whether that commit has an image; both
  are recorded, outside the checkout, for [`status`](artefacts.md#status).
  Nothing is downloaded or extracted. A commit with no image fails the entry.

Nothing that `fetch` does can change which commit is checked out — it is safe to
run in a dirty workspace.

## pull

Put every checkout where
[resolution](recursive-dependencies.md#how-a-revision-is-chosen) says it goes,
against the remotes as they are now — including a revision a dependency starts
asking for in this very pull, and implicit dependencies new to the graph.
Anything missing is checked out first.

```
gitscale pull                   # everything
gitscale pull imports/core      # one entry
```

In order:

1. **Tidy the stores**: repair checkouts the root's move broke, and prune what
   deleted root worktrees left — see [moving and deleting](stores.md#moving-and-deleting).
2. **Carry** the topic, when the root's branch was just created from another
   topic — see [a new branch from a topic](topics.md#a-new-branch-from-a-topic).
3. **Resolve**, fetching each store once.
4. **Place** each checkout, below.
5. **Link** the [recursive dependencies](recursive-dependencies.md).
6. **Prune** images nothing has used lately, at most once a day — see
   [images](stores.md#images).
7. Run the [`post_sync` hook](hooks.md#post_sync), if one is configured.

| Entry | What happens |
|---|---|
| Missing directory, or an empty one | A worktree of its store, detached at its commit, read-only |
| Off the topic | Detached at the commit its revision resolves to, read-only. A [branch](dependencies.md#branch) revision moves to the new tip |
| On the topic | On the topic branch, writable, fast-forwarded to its upstream — see [where each checkout goes](topics.md#where-each-checkout-goes) |
| `replace` artefact | Nothing to do, and nothing asked of the registry, when the revision still names the installed commit. Otherwise the new image is downloaded — only the layers the store does not hold — then the files are replaced. A commit with no image fails and leaves the installed files alone |
| `overlay` artefact | Placed as a git checkout, then the image of its commit laid over it |
| Directory holding files but no repository | `FAIL … exists but holds no git repository`. Left as it is — [`gitscale clean -f`](clean.md#a-directory-holding-no-repository) removes it, or move it aside, and run again |
| A checkout that is not a worktree of its store | `FAIL … not a gitscale worktree`, left alone — see [checkouts GitScale did not make](stores.md#checkouts-gitscale-did-not-make) |
| Symlink planted by an enclosing workspace (running inside a child repository) | `skip (symlink)` — that checkout and its revision belong to the outer workspace's root. Named, it is unlinked |
| Any other symlink | Removed and replaced by a checkout |
| Artefact entry, no registry known for its host | `FAIL … no registry is known for …`, naming the [`[registries]`](configuration.md#registries) entry to add |

A checkout is **moved only when nothing can be lost**. Uncommitted changes to
tracked files fail the entry with `not moved: uncommitted changes; commit or
stash, then pull again`; so do commits made at a detached HEAD that no branch,
tag or remote holds, with the command to keep them. Untracked files do not
block a move: git keeps them, and refuses rather than overwrite one. Commits on
a topic branch never block a move: they stay on the branch in the store.

In CI, where checkouts hold nobody's work, the move is forced, and every
checkout pulled is then cleaned with [`gitscale clean -f`](clean.md) — tracked
files are updated in place, so unchanged files keep their mtimes and a restored
build cache stays valid.

An entry that fails — a remote that cannot be reached, a move refused, an
artefact whose pipeline has not published yet — does not stop the others:
every other checkout is still pulled and linked, and then `pull` exits non-zero
naming how many failed.

This is also the command an [installed git hook](hooks.md#git-hooks) runs, which
is what makes a fresh clone, a `git switch` or a new worktree populate itself.

## push

```
gitscale push                   # everything on the topic
gitscale push imports/core      # one entry
```

Push the topic branch of the root and of every checkout on it, as `git push -u
origin <branch>`: the remote branch gets the same name and becomes the
upstream. Checkouts off the topic sit at a pin and are skipped (`not on the
topic`), as are artefacts, directories that hold no checkout and symlinked
(deduped) entries. Off a topic, nothing is pushed — the root included. Inside
CI, the remote is repointed at the job-token HTTPS URL first where that
applies — see [CI authentication](ci-authentication.md).

## sync

The one-command "make the workspace match the config", in this order:

1. **pull** everything.
2. **Relink** — restore [recursive dependency](recursive-dependencies.md)
   symlinks that have been replaced by real checkouts, remove checkouts nothing
   needs any more, and remove orphaned links.
3. **push** everything on the topic.
4. Run the [`post_sync` hook](hooks.md#post_sync).

```
gitscale sync                   # everything
gitscale sync imports/core      # one entry
gitscale sync --force           # also relink modified checkouts, remove valid-target orphans
```

Every step runs even when an earlier one failed for some entry, so one
repository that cannot be pulled does not leave the rest unlinked or unpushed.
`sync` then exits non-zero with the first failure, and the `post_sync` hook
runs only when every step succeeded.

Step 2 happens *before* the push so that local hygiene is not blocked by a
remote or authentication failure. It is also the step that can refuse: an
unlinked checkout with local work, a checkout nothing needs but with local
work, or an orphaned link whose target still resolves, is reported and left in
place, and `sync` exits non-zero asking for `--force`. Broken orphans are
always removed.

**A checkout nothing needs any more** is one whose entry you removed or
renamed in `.gitscale.toml`, or an implicit dependency no repository asks for
now. A `sync` given no names removes it when that loses nothing — no
uncommitted changes, unpushed commits or stash, while untracked symlinks (the
links gitscale planted) and files git ignores go with it; an artefact never
holds any work, since every pull replaces it whole. `pull` keeps a record of
the checkouts it manages — `gitscale/checkouts.json` in the root worktree's
git directory — and only those are ever removed: a directory gitscale did not
make is never touched.

## commit

Commit across the workspace with one shared message.

```
gitscale commit -m "price cache"
gitscale commit -m "wip" imports/core apps/web
```

In each selected checkout on the [topic](topics.md) this is `git add -A`
followed by `git commit -m`, so untracked files are included. A checkout off
the topic is detached at a pin and never committed; one with changes says how
to bring it in:

```
  skip  imports/e (not on topic feat/x; run gitscale develop imports/e)
```

Also skipped: artefact entries, directories that hold no checkout, symlinked
(deduped) entries — the real checkout is committed under its own name — and any
repo whose working tree is already clean.

When run with no names, the root repository is committed too, reported as `.`.
It is never committed when specific names were given. Off a topic, only the
root is committed.

Nothing is pushed. Follow with `gitscale push` or `gitscale sync`.

Because the root commit is a `git add -A`, the checkout directory should be in
the root's `.gitignore` — see
[ignoring the checkout directory](dependencies.md#ignoring-the-checkout-directory).

## Choosing between them

| You want to | Use |
|---|---|
| Set up a workspace | `git clone <url>`, with a [git hook](hooks.md#git-hooks) installed; else `git clone` then `gitscale pull` |
| Materialise entries added to the config | `gitscale pull` |
| See what changed upstream, safely | `gitscale fetch` then `gitscale status` |
| Get up to date | `gitscale pull` |
| Get up to date, fix links, and push | `gitscale sync` |
| Change a dependency | `git switch -c <topic>`, then [`gitscale develop <dir>`](topics.md#gitscale-develop) |
| Work on two things at once | `git worktree add` of the root — see [parallel topics](topics.md#parallel-topics) |
| Turn a real checkout back into a deduped symlink | `gitscale sync` (`--force` if it has local work) |

---

[← 2.3 Recursive dependencies](recursive-dependencies.md) · [Contents](README.md) · [Next → 2.5 Stores, worktrees and the CI cache](stores.md)

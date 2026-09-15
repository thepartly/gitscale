# GitScale

Manage multiple sub-repositories from a single config file. An alternative to git submodules — simpler, with readonly enforcement, artefact mode, and cloud storage for pre-built archives.

## Install

```
cargo install gitscale
```

Requires a stable Rust toolchain. To build from a checkout instead:

```
git clone https://github.com/thepartly/gitscale.git
cd gitscale && cargo install --path .
```

## Quick start

Create a `.gitscale.toml` in your project root:

```toml
[repos]
"libs/core" = { url = "https://github.com/org/core.git", revision = "main", mode = "readonly" }
"libs/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
```

Then clone everything:

```
gitscale clone
```

Check status:

```
gitscale status
```

```
      REPO          PATH   MODE        REF    EXPECTED   STATUS
✔     libs/core     -      readonly    main   main       ok
✔     libs/utils    -      readwrite   v2.1   v2.1       ok
⤷     libs/shared   ../s   readonly    main   main       symlink
```

Objects are fetched once per machine rather than once per workspace: see
[Object cache](#object-cache).

## Config file

The `.gitscale.toml` file lives at the root of your project. GitScale searches upward from the current directory to find it.

### Repos

```toml
[repos]
"libs/core" = { url = "git@github.com:org/core.git", revision = "main", mode = "readonly" }
"libs/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
"meta/frontend" = { url = "https://github.com/org/frontend.git", revision = "main", mode = "artefact" }
```

Each entry maps a local directory to a git repo:

- **url** (required) — Git repository URL (HTTPS or SSH)
- **revision** — Branch, tag, or commit SHA. Defaults to the repo's default branch. See [Shallow clones](#pinning-to-a-commit-sha) for how SHAs are handled
- **mode** — Access mode:
  - `readwrite` (default) — Normal clone, full access
  - `readonly` — Cloned, but all files have write permissions removed
  - `artefact` — No git clone. Artefact archive synced via cloud storage
- **recursive** — Read the repo's nested `.gitscale.toml`: resolve its transitive deps, and clean it according to its own `[clean]` rules. With `recursive = false` gitscale does not read that config, and `clean` skips the repo rather than cleaning it blind (default: `true`)

### Storage

Configure cloud storage for artefact data:

```toml
[storage]
url = "https://my-bucket.s3.us-east-1.amazonaws.com/gitscale"
```

See [Artefact storage](#artefact-storage) below.

### Share

```toml
[share]
dissociate = true
```

Controls how a clone reuses a copy of a sub-repository already on this machine.
See [Reusing a local copy](#reusing-a-local-copy) below. Off by default.

### Cache

The object cache, on by default:

```toml
[cache]
enabled    = true
dir        = "~/.local/share/gitscale"
dissociate = false
adopt_root = false
```

See [Object cache](#object-cache) below.

### Clean

What `gitscale clean` keeps:

```toml
[clean]
exclude = [".vscode", ".idea", ".env", "envs/", "tmp"]
```

Patterns use `.gitignore` syntax and are anchored at the root of the repo whose
config they appear in — not at the workspace root. So `tmp` matches at any
depth, `/tmp` only at the top, and `envs/` only directories.

**A `[clean]` table speaks for its own repo and nothing below it.** The root's
exclusions apply to the workspace repo's own working tree; a sub-repository
declares its own in the `.gitscale.toml` in its checkout. A repo that is not a
workspace can carry a config with nothing but a `[clean]` table in it:

```toml
# core/.gitscale.toml — core keeps its own scaffolding
[clean]
exclude = ["envs/", ".env"]
```

This is deliberate. A sub-repository knows what its build leaves behind; a root
config enumerating that on its behalf goes stale the moment the sub-repository
changes. For a repo you cannot add a config to, pass `--exclude` on the command
line instead — it applies to every repo cleaned.

Reading a sub-repository's `[clean]` is gated on its `recursive` flag, like
every other nested-config read. A repo with `recursive = false` is skipped by
`clean` entirely: with its keep-list out of reach, leaving it alone beats
cleaning it with no idea what it wanted kept.

## Commands

### `gitscale clone [NAMES...]` / `gitscale clone URL [DIRECTORY]`

Clone sub-repositories that don't exist locally yet. Artefact entries are downloaded from cloud storage.

```
gitscale clone                  # clone all
gitscale clone libs/core        # clone one
```

Given a repository URL instead, it bootstraps a whole workspace from cold:
clone the root repository, then everything its `.gitscale.toml` declares.

```
gitscale clone https://github.com/org/root.git         # into ./root
gitscale clone git@github.com:org/root.git my-ws       # into ./my-ws
```

The root repository goes through the [object cache](#object-cache) like any
other, so the second workspace of it costs nothing over the wire. A first
argument is read as a URL when it has a scheme, is an scp-style SSH address, or
is an absolute path — declared directories are always relative, so the two
cannot be confused.

### `gitscale fetch [NAMES...]`

Fetch latest remote state without modifying local files. For git repos: `git fetch`. For artefacts: HEAD request to check remote ETag.

```
gitscale fetch                  # fetch all
gitscale fetch libs/core        # fetch one
```

### `gitscale pull [NAMES...]`

Pull latest changes. For git repos: `git pull --ff-only`. For artefacts: downloads from cloud storage if remote is newer. Clones repos that don't exist yet.

```
gitscale pull                   # pull all
gitscale pull libs/core         # pull one
```

### `gitscale push [NAMES...]`

Push local changes. For git repos: `git push`. Readonly and artefact entries are skipped.

```
gitscale push                   # push all
gitscale push libs/core         # push one
```

### `gitscale sync [NAMES...]`

Full sync: clone + pull + push in sequence. After syncing, it also:

- **Reconciles remotes** — updates each clone's `origin` URL to match the configured URL (e.g. after switching from HTTPS to SSH).
- **Relinks** — restores symlinks for recursive dependencies. A clone that replaced an expected symlink is relinked automatically when clean; if it has local modifications, use `--force`.
- **Removes orphaned symlinks** — leftover gitscale symlinks whose dependency was removed from config. Broken orphans (target missing) are removed automatically; orphans whose target still resolves require `--force`.

```
gitscale sync                   # sync all
gitscale sync libs/core         # sync one
gitscale sync --force           # also relink modified clones / remove valid-target orphans
```

### `gitscale clean [NAMES...]`

Remove untracked files from the workspace repo and each sub-repository, the way
`git clean -xd` does in one repo. Without `-f` it only lists what would go.

```
gitscale clean                  # dry run: list what would be removed
gitscale clean -f               # remove it
gitscale clean -f core          # just this repo
gitscale clean -f .             # just the workspace repo
gitscale clean -f -e 'dist/'    # keep dist/ in every repo cleaned
```

Each repo is cleaned with its own exclusions — see [Clean](#clean) above.
Beyond those, clean always keeps:

- **The declared checkouts**, at every level. A sub-repository directory is an
  untracked directory to the repo holding it, so an unguarded `git clean -xdf`
  at the root deletes the whole workspace — and one declared inside another
  repo (`core` and `core/vendor`) goes the same way when that repo is cleaned.
  Clean excludes them wherever they sit.
- **Managed symlinks.** The links `resolve` plants for [recursive
  dependencies](#recursive-dependencies) are untracked files to git. Orphaned
  gitscale symlinks are *not* protected — those are removed, as `sync` would.
- **`.gitscale.toml`**, even when it has not been committed yet.

Repos are skipped, not cleaned, when they are `recursive = false` (keep-list
out of reach), artefact mode (no working tree), not cloned, or a symlink to a
checkout outside the workspace. The dry run names the reason for each.

Cleaning runs top down: the workspace repo first, then each declared
checkout once. A repo reachable both as a checkout and through a recursive
dependency's symlink is still cleaned once, on its own rules.

Nested git repositories gitscale does not manage — a clone someone made by
hand, say — are reported rather than deleted: clean does not pass `git clean`'s
second `-f`. Remove those by hand.

### `gitscale status`

Show the status of all managed repos. Use `--fetch` to check remote state before reporting.

```
gitscale status                 # table output
gitscale status --format json   # JSON output
gitscale status --fetch         # fetch before checking
```

Columns:

| Column | Description |
|--------|-------------|
| REPO | Directory name of the managed repo |
| PATH | Where a symlinked entry points, or `-` for an ordinary checkout. Only [recursive dependencies](#recursive-dependencies) deduped into one checkout are symlinks |
| MODE | `readonly`, `readwrite`, or `artefact` |
| REF | Current git ref (branch/tag/SHA) |
| EXPECTED | Declared revision from `.gitscale.toml` |
| STATUS | Flags describing repo state |

Status icons and flags:

| Icon | Color | Flag | Meaning |
|------|-------|------|---------|
| `✔` | green | **ok** | Clean, on expected ref |
| `⤷` | cyan | **symlink** | Resolved as symlink to parent-level checkout |
| `~` | yellow | **unlinked** | Expected symlink replaced by real clone (safe to relink) |
| `~` | red | **unlinked, modified** | Unlinked clone has local changes (unsafe to relink) |
| `~` | red | **unlinked, dirty** | Unlinked clone is clean, but the parent repo has uncommitted changes  (unsafe to relink) |
| `⊘` | yellow | **orphan** | Leftover gitscale symlink whose dependency was removed from config (target still resolves) |
| `⊘` | red | **orphan, broken** | Leftover gitscale symlink whose target no longer exists |
| `!` | red | **dirty** | Uncommitted changes |
| `≠` | red | **ref-mismatch** | On a different branch than declared |
| `≠` | red | **stale** | Shallow clone: local differs from upstream |
| `!` | red | **cache-broken** | Borrows objects from a cache entry that is gone — run [`gitscale cache repair`](#gitscale-cache-statusupdaterepaircompact) |
| `⇑` | yellow | **+N** | Commits ahead of upstream |
| `⇓` | yellow | **-N** | Commits behind upstream |
| `⇅` | yellow | **+N, -N** | Diverged (ahead and behind) |
| `◆` | cyan | **detached** | HEAD is detached |
| `✘` | red | **missed** | Directory doesn't exist yet |

### `gitscale add DIRECTORY REPO_URL REVISION`

Add a new entry to `.gitscale.toml`.

```
gitscale add libs/core https://github.com/org/core.git main
gitscale add libs/core https://github.com/org/core.git main --mode readonly
gitscale add meta/svc https://github.com/org/svc.git main --mode artefact
```

### `gitscale remove DIRECTORY`

Remove an entry from `.gitscale.toml`.

```
gitscale remove libs/core
```

### `gitscale cache <status|update|repair|compact>`

Own the [object cache](#object-cache) directly. Everything else touches it
implicitly; these are for when that is not enough.

```
gitscale cache status                        # what the cache holds, entry by entry
gitscale cache update                        # warm entries nobody has pulled yet
gitscale cache update libs/core              # just this one
gitscale cache repair                        # re-create entries this workspace borrows from
gitscale cache compact                       # repack, evict what nothing used in a month
gitscale cache compact --keep-recent 2weeks
```

`status` is one line per **repository**, named after the directory that declares
it where this workspace declares one, with what each kind of entry costs it — a
machine that both develops and runs jobs has a mirror and a snapshot for the
same repo, and pays for both:

```
cache  /home/dev/.local/share/gitscale  (4 entries, 104.8 KiB)

  REPO          MIRROR  SNAPSHOTS     TOTAL  REVS  LAST USED
  libs/core   26.2 KiB          -  26.2 KiB     9  2 hours ago
  libs/utils         -   26.2 KiB  26.2 KiB     2  just now
      c6e8981d205b  just now
      73739376c62a  6 days ago
```

| Column | Meaning |
|--------|---------|
| MIRROR | The mirror entry for this repository, or `-` if it has none |
| SNAPSHOTS | The snapshot entry, holding every pinned commit CI has asked for |
| TOTAL | Both together — what this one repository costs the cache |
| REVS | Cached revisions: a mirror's branches and tags, a snapshot's pins |
| LAST USED | When anything last read either entry |

MIRROR and SNAPSHOTS are whole entries, not the revision checked out: one
mirror holds every branch and tag the remote has, one snapshot holds every
pinned commit, and a machine that both develops and runs jobs has one of each.
The line above the table is the grand total across every entry, so the TOTAL
column adds up to it.

Pins are listed beneath their row, each with its own age, because that is what
`compact` drops one at a time. A mirror's refs are counted in REVS but listed
only under `-v`: a busy repository has hundreds of them, and that is not a
summary.

`compact` works from anywhere: the cache belongs to the user, not to any one
workspace. A `.gitscale.toml` is used when there is one, so `[cache] dir` is
honoured.

### `gitscale hook <install|uninstall|status|run>`

Install gitscale as a git hook so `gitscale pull` runs automatically whenever a
checkout or merge changes the working tree. See [Git hooks](#git-hooks).

```
gitscale hook install --local     # this repository only (default)
gitscale hook install --global --allow 'github.com/acme/*'   # every repo for the current user
gitscale hook install --system --allow 'github.com/acme/*'   # every repo for every user
gitscale hook status              # where hooks are installed, what they allow, what shadows them
gitscale hook uninstall --local
```

`--allow` takes comma-separated glob patterns naming the repositories whose
`[hooks]` commands the installed hook may run. It is required for `--global`
and `--system`; see [hook allowlist](#hook-allowlist).

### Global options

- `-v, --verbose` — Enable verbose output (must go before the subcommand)
- `--no-cache` — Talk to remotes directly, ignoring the [object cache](#object-cache)
- `--version` — Show version
- `-C, --root PATH` — Override the root directory (available on all subcommands)

```
gitscale -v sync
gitscale --version
gitscale -C /path/to/project status
```

## Repo modes

### readwrite (default)

Normal git clone. Full read/write access to files.

### readonly

Shallow-cloned (`--depth 1`) locally, then all file write permissions are stripped (excluding `.git/`). Git operations (sync, pull) use `fetch --depth 1` + `reset --hard` to update.

Useful for vendored dependencies you shouldn't modify.

### artefact

No git clone. An artefact archive (`<revision>.tar.gz`) is downloaded from cloud storage and extracted into the entry's directory on `clone` and `pull`. All extracted files are kept readonly. ETag-based change detection avoids unnecessary transfers. Push is not supported (artefacts are published by CI).

The archive content is opaque to gitscale — it can contain anything.

## Shallow clones

Readonly repos are always shallow-cloned (`--depth 1`). This is fast and disk-efficient — only the declared revision is fetched.

When the `CI` environment variable is set to `1` or `true` (as done by GitHub Actions, GitLab CI, etc.), **all** git repos are shallow-cloned, regardless of mode.

| Context | readwrite | readonly | artefact |
|---------|-----------|----------|----------|
| Local, `--no-cache` | full clone | shallow | no git |
| Local, cached | full clone | full clone | no git |
| CI (`CI=1`) | shallow | shallow | no git |

With the [object cache](#object-cache) on — the default — depth follows the
kind of cache entry serving the repo rather than the mode. A developer machine
borrows from a full mirror, where shallow buys nothing and borrows less
cleanly, so a readonly repo is cloned whole and its history is there to browse
at no extra cost. CI is served by shallow snapshot entries and keeps the
depth-1 checkout it has today.

Shallow repos show `≠ stale` in status when the local commit differs from upstream (exact behind count is unavailable).

### Pinning to a commit SHA

A `revision` naming a branch or tag is fetched with `--depth 1 --branch <revision>`, leaving the clone on that branch.

A `revision` naming a **commit SHA** cannot use `--branch`, which accepts only branch and tag names. GitScale initialises the repo and fetches that single commit instead:

```bash
git init && git remote add origin <url>
git fetch --depth 1 origin <sha>
git checkout --detach FETCH_HEAD
```

The result is a shallow clone with a **detached HEAD** at exactly the pinned commit. This matters most in CI, where every repo is shallow — a SHA-pinned entry that clones fine locally would otherwise fail there.

Some git servers refuse to serve an arbitrary commit. If the fetch is rejected, GitScale falls back to a full clone and checks the revision out normally. GitHub and GitLab both permit it.

**Trade-off:** a `revision` of 7–64 hexadecimal characters is treated as a commit SHA. A branch or tag whose name happens to be entirely hexadecimal (e.g. `abcdef1`) therefore takes the SHA path as well. It still resolves and checks out the right commit, but the clone ends up detached rather than on the branch — rename the ref or use a longer name if you need to stay on it.

## Reusing a local copy

A second worktree of a workspace, or a workspace cloned with `--reference`,
sits beside one that already downloaded every sub-repository. GitScale finds
that earlier workspace and clones each sub-repository from it with
`git clone --reference`, so the objects are copied off local disk instead of
fetched over the network.

This matters most with [git hooks](#git-hooks) installed: `git worktree add`
fires `post-checkout`, which pulls, which clones every declared repository into
the new worktree. That is the command it makes fast.

Nothing is shared implicitly — git has no way to connect a fresh clone to a
sibling, and its worktree metadata says nothing about the repositories declared
inside a workspace. GitScale works the source out itself:

| How the workspace was made | What identifies the source |
|---|---|
| `git worktree add` | the main worktree, from `git worktree list` |
| `git clone --reference` | `objects/info/alternates` |
| an ordinary clone | nothing — sub-repositories clone normally |

The source is always the **main** worktree, never a sibling. Sibling worktrees
are routinely deleted once their branch merges, and an alternate that
disappears leaves the borrowing repository unable to read its own history.

A copy is only borrowed from when its `origin` matches the configured `url`.
Occupying the same relative path is not enough: two unrelated workspaces may
both keep something at `libs/core`. Symlinked paths (those
[`resolve` creates](#recursive-dependencies) to dedupe a recursive dependency)
and artefact entries are skipped.

Anything unsuitable falls through to the [object cache](#object-cache), which
covers exactly these misses — a repo the source workspace does not have, or a
workspace that came from nowhere in particular.

### dissociate

```toml
[share]
dissociate = true
```

By default the new clone keeps a pointer to the source and does not copy its
objects, which saves disk as well as network. With `dissociate = true` the
borrowed objects are copied in and the pointer is dropped once the clone is
made: the network saving remains, the disk saving does not, and the result no
longer depends on the source.

Turn it on where the source may be moved, deleted or garbage-collected. `git gc`
in the source repository does not know it has borrowers and can delete objects
one still needs — nothing warns when that happens, and the borrowing clone is
left unable to read its own history.

## Object cache

A bare mirror per repository URL under `~/.local/share/gitscale`, borrowed from
with `git clone --reference`. **On by default**; `--no-cache` or
`[cache] enabled = false` opts out.

One rule governs the direction: **the cache talks to the remote, the workspace
talks to the cache.** Every clone, fetch and pull updates the cache entry
first, then builds or updates the workspace from it:

```
1.  git -C <cache> remote update -p              # the only network call
2.  git -C <workspace> fetch <cache> '+refs/heads/*:refs/remotes/origin/*'
```

Step 2 is local and free. N workspaces on the machine share step 1 — that is
the whole saving, and it is why the entry is updated even when the workspace
could have fetched for itself. Nothing ever flows back from a workspace, there
is no timer and no TTL, and nothing depends on anyone running a maintenance
command.

`origin` in the workspace stays the real URL, so pushes and a hand-run
`git fetch` still go where they always did; only gitscale's own refresh reads
from the cache. Refs always come from the remote or from an entry that has just
been updated from it, so **a stale cache changes how many bytes cross the wire,
never which commit you land on**.

Object sources are tried in order: the [source workspace](#reusing-a-local-copy)
when it has the repo, then the cache entry, then the remote. A repo borrowed
from a source workspace keeps that workspace as its alternate, and its cache
entry is still updated on each pull — so a later workspace that has to fall
back to the cache finds it current.

| | Source workspace only | Cache | CI runner |
|---|---|---|---|
| First workspace on the machine | full clone per repo | full clone, lands in cache | full clone, lands in cache |
| Each later workspace | free only for repos the source has | free | free |
| `gitscale pull` | delta per repo **per workspace** | delta per repo **per machine** | none, unless the head moved |
| SHA-pinned repos | full shallow fetch every time | free once cached | free once cached |
| N cold starts at once | N downloads | 1 | 1 |

### Per-user, and only per-user

The cache lives in the invoking user's own data directory. There is no
system-wide scope and no shared directory gitscale will create.

A mirror several users write through is a machine-wide blast radius: one
poisoned entry reaches every workspace on the box. That is the reach
`gitscale hook install --system` earns only by naming an explicit
[`--allow`](#hook-allowlist), and something that is on by default cannot ask for
it. `dir` still points wherever you say, including somewhere shared by other
means — that is your arrangement to make, not a mode gitscale sets up.

The location is resolved in this order:

| | |
|---|---|
| `[cache] dir` | as written, with a leading `~/` expanded |
| `GITSCALE_CACHE_DIR` | used as-is |
| `XDG_DATA_HOME` | `$XDG_DATA_HOME/gitscale` |
| otherwise | `~/.local/share/gitscale` |

A workspace bootstrapped with `gitscale clone <url>` has no config to read yet,
so its root repository uses the environment alone.

### Two kinds of entry

Developer machines and CI want opposite things, so the cache holds both, and
which one is used is decided by the environment rather than by the revision:
CI takes snapshots, developer machines take mirrors.

**Mirror** — `mirror/<entry>.git`. Full history, updated from the remote before
each workspace operation, consumed with `--reference`. History to browse, and
objects shared across every workspace on the machine.

**Snapshot** — `snapshots/<entry>.git`. A shallow bare repo holding each pinned
commit as `refs/heads/pin/<sha>`, consumed by an ordinary **local clone** after
which `origin` is repointed at the real URL. That is the depth-1 checkout CI
already uses, served from local disk.

A SHA-pinned repo needs no network to resolve. A branch or tag costs one
`git ls-remote` — a ref advertisement, no objects; if the head has not moved,
the commit is already an entry ref and nothing further transfers.

The two consume their entries differently, and it matters: a job **copies**
what it takes, so evicting an entry — or deleting the whole cache — under a
running job is harmless. Git also refuses a shallow repository as a
`--reference`, so copying is not merely the safer choice for CI, it is the only
one.

Artefact entries are never cached: an unpacked archive has no object store.

### CI

The cache being on by default is what makes a shell runner with a persistent
home directory work. The runner clones the root repository itself, so there is
no source workspace to inherit from:

| Action | Network |
|---|---|
| First job ever on the machine | one full download per repo |
| Every later job | none for sub-repos |
| SHA-pinned repo the cache has | none |
| N parallel jobs, cold | one download total, not N |
| Job ends, workspace wiped | the cache keeps everything the job fetched |

That is one user on one machine — the sharing CI needs is between jobs, not
between users.

### Keeping it honest

`gitscale cache status` is where the cache reports itself — where it is, how
big, and what each repository costs it. `gitscale status` stays about the
workspace and says nothing about the cache, with one exception: a checkout
borrowing from an entry that has been deleted is flagged **cache-broken**,
because it cannot read its own history and nothing else would tell you.

`gitscale status --format json` carries the rest per repository, for anything
that wants to see it without a column in the way:

| `cache` | |
|---------|---|
| `-` | Nothing cached for this repository, and the checkout owns its objects |
| `mirror` | Borrowing from a mirror entry |
| `snapshot` | Built from a snapshot entry that holds this exact commit — CI's flow |
| `workspace` | Borrowing from a [source workspace](#reusing-a-local-copy) instead |
| `copy` | The checkout owns its objects, but an entry exists for the repository anyway |
| `broken` | Borrowing from something that is no longer there |

`copy` is what every checkout made before this machine had a cache looks like,
and it is not a fault. `--reference` is a decision taken when a repository is
cloned and nothing re-links a clone afterwards, so the objects sit in both
places. The entry is still updated on every pull and still serves every other
workspace on the machine — only *this* checkout pays for its own copy, and only
in disk. Re-cloning it is all it takes to borrow instead.

`snapshot` is the one claim here that is not a link on disk: a CI job clones
locally out of a snapshot entry, which copies the objects. What is reported is
that the entry holds the commit this checkout is on — which is what decides
whether the next job pays for it again.

`gitscale cache compact` repacks every entry and evicts the ones nothing has
used for `--keep-recent`, a month by default. Each entry carries a last-used
marker, touched on every hit, and each pinned commit carries its own — so a
snapshot entry in daily use can still shed the pins that have gone cold.

Mirrors are repacked but never pruned. `repack -d` is the one operation that can
delete objects a live borrower still needs, and nothing warns when it does;
`gc.auto = 0` is set on every entry at creation for the same reason. Snapshots
have no borrowers, so those are pruned properly.

A workspace whose entry is deleted cannot read its own history, and git reports
nothing until something tries to read an object. `gitscale cache repair`
re-creates the entries a workspace borrows from:

```
gitscale cache repair
  repair libs/core
  ok     libs/utils
Repaired 1 entry.
```

### Copying instead of borrowing

```toml
[cache]
dissociate = true
```

As with [`[share] dissociate`](#dissociate): the borrowed objects are copied in
and the link dropped once the clone is made. The network saving remains, the
disk saving does not, and the result no longer depends on the entry surviving.

### Adopting a root repository

```toml
[cache]
adopt_root = true
```

A root created by `gitscale clone <url>` is linked to the cache from birth. One
cloned by plain `git clone` holds its own full copy and shares nothing. With
`adopt_root`, the next `gitscale clone` or `pull` seeds an entry from it (local,
no network), points it at that entry, and reclaims the duplicate objects —
measured on a 60-commit root, 1244 KB down to 20 KB, history intact.

**Opt-in, and it stays opt-in.** The reclaim step deletes the repository's own
objects and converts something that stood on its own into something that depends
on the cache. Recoverable with `cache repair`, but it is your call, not a
default. It is not needed in CI: the runner clones the root itself and the
workspace is discarded at the end.

## CI authentication

CI runners have a token for the forge they run on, but no SSH key. An entry declared as `git@gitlab.example.com:group/repo.git` clones fine on a laptop and fails inside a job.

GitScale derives the fix from the job environment. On **GitLab CI** (`CI_JOB_TOKEN` plus `CI_SERVER_URL`, or `CI_SERVER_HOST`/`CI_SERVER_PROTOCOL`/`CI_SERVER_PORT`) and on **GitHub Actions** (`GITHUB_ACTIONS` plus `GITHUB_TOKEN` or `GH_TOKEN`, with `GITHUB_SERVER_URL`), every entry **hosted on that same server** is fetched over HTTPS with the job token instead of over SSH:

```toml
"libs/payments" = { url = "git@gitlab.example.com:acme/payments.git", revision = "main" }
```
```
# inside a GitLab job, with no pipeline setup:
https://gitlab.example.com/acme/payments.git   authenticated as gitlab-ci-token
```

Both SSH spellings are recognised — `git@host:group/repo.git` and `ssh://git@host/group/repo.git` — and nested subgroups are preserved. An existing clone whose `origin` still points at SSH (a cached workspace, or the runner's own checkout) is repointed before the next fetch.

Nothing else is touched:

- **Other hosts are never offered the token.** A dependency on a different server is fetched exactly as configured. Its host has to match the CI server's for the credential to apply at all.
- **The token never lands on disk or in argv.** GitScale passes git a credential helper scoped to the CI server that reads the token variable when git asks for the password, so only the *variable name* appears in the command line.
- **Nothing sensitive is persisted.** The credential setup lives on the git command line for the duration of the call. The only thing written to `.git/config` is the plain HTTPS remote URL, which carries no credentials.

Set `GITSCALE_NO_CI_AUTH=1` to switch this off and fetch exactly what the config says.

### GitLab job token allowlists

Detection cannot grant access. On GitLab, the target project must list the calling project under **Settings → CI/CD → Job token permissions**, or the clone returns 403 no matter how it authenticates. GitScale spells this out when it sees one:

```
FAIL  libs/payments: fatal: unable to access 'https://gitlab.example.com/acme/payments.git/': The requested URL returned error: 403
hint: the GitLab job token (CI_JOB_TOKEN) was rejected. Add this project to the target project's Settings -> CI/CD -> 'Job token permissions' allowlist, or give the job a token with read access.
```

### GitHub Actions token scope

GitHub has no equivalent of the allowlist. The built-in `GITHUB_TOKEN` is a freshly minted installation token for the GitHub Actions app, and that installation is scoped to exactly **one repository** — the one the workflow lives in. It expires when the job ends.

| Entry | Result |
|-------|--------|
| The workflow's own repository | works |
| Any public repository | works (readable without credentials anyway) |
| A **private** repository, even in the same org | 403 |

The workflow's `permissions:` block only widens or narrows *which scopes* the token holds on its own repository (`contents`, `packages`, `id-token`, …). It cannot extend the token to a second repository. `contents: read` is simply the scope a clone needs.

For a private cross-repo dependency, supply a token that does cover it:

| Option | Notes |
|--------|-------|
| GitHub App token | Install an app on both repositories and mint a short-lived token in the job. No user account involved, expires within the hour. |
| Fine-grained PAT | Grant **Contents: Read** on the specific repositories, store as a repo or org secret. Tied to a user account. |
| Deploy key | An SSH key registered on the target repository. Per-repo, and keeps you on SSH rather than HTTPS. |

GitScale reads whichever token is in `GITHUB_TOKEN` or `GH_TOKEN` and does not care where it came from, so exporting a better token under that name is the whole change:

```yaml
- uses: actions/create-github-app-token@v1
  id: app-token
  with:
    app-id: ${{ vars.APP_ID }}
    private-key: ${{ secrets.APP_PRIVATE_KEY }}
    owner: acme
- run: gitscale sync
  env:
    GITHUB_TOKEN: ${{ steps.app-token.outputs.token }}
```

## Artefact storage

### Setup

Add a `[storage]` section to your config:

```toml
[storage]
url = "https://my-bucket.s3.us-east-1.amazonaws.com/gitscale"
```

### Supported backends

Any S3-compatible storage works with the same URL pattern:

| Service | Example URL |
|---------|------------|
| AWS S3 | `https://my-bucket.s3.us-east-1.amazonaws.com/prefix` |
| MinIO | `https://minio.corp.com:9000/my-bucket/prefix` |
| Cloudflare R2 | `https://<account>.r2.cloudflarestorage.com/my-bucket/prefix` |
| Backblaze B2 | `https://s3.us-west-004.backblazeb2.com/my-bucket/prefix` |
| DigitalOcean Spaces | `https://nyc3.digitaloceanspaces.com/my-bucket/prefix` |
| Google Cloud Storage | `https://storage.googleapis.com/my-bucket/prefix` |
| Local directory | `/tmp/artefacts` or `file:///home/user/artefacts` |

### Authentication

Set credentials via environment variables (not needed for local storage):

**S3-compatible** (AWS, MinIO, R2, B2, Spaces):
```
export AWS_ACCESS_KEY_ID=your-key
export AWS_SECRET_ACCESS_KEY=your-secret
export AWS_DEFAULT_REGION=us-east-1   # optional, detected from URL
```

**Google Cloud Storage:**
```
export GOOGLE_TOKEN=$(gcloud auth print-access-token)
```

### Object layout

Artefact archives are stored at: `{storage_url}/{host}/{owner}/{repo}/{revision}.tar.gz`

For example, with `url = "https://bucket.s3.amazonaws.com/meta"` and a repo at `https://github.com/org/app.git` on revision `main`:

```
https://bucket.s3.amazonaws.com/meta/github.com/org/app/main.tar.gz
```

## Config hooks

Add a `[hooks]` section to run commands after certain operations:

```toml
[hooks]
post_sync = "make install"
on_pull_error = "warn"
```

- **post_sync** — Runs after `pull` and `sync` complete (executed via `sh -c` in the config root directory). Fails the command if the hook exits non-zero. When a [git hook](#git-hooks) triggers it, only for repositories on that hook's [allowlist](#hook-allowlist).
- **on_pull_error** — What a [git-hook-triggered](#git-hooks) pull should do when it fails: `"fail"` returns non-zero, failing the git operation; `"warn"` reports and lets it succeed. Defaults to `"fail"` under CI and `"warn"` otherwise.

Not to be confused with [Git hooks](#git-hooks) below, which is how gitscale hooks *into git*.

### Hook allowlist

`.gitscale.toml` lives *inside* the repository, so its `[hooks]` table is
written by whoever wrote the branch — including someone who has only opened a
merge request. A `--global` or `--system` [git hook](#git-hooks) turns every
`git clone` and `git checkout` on the machine into a trigger for it: cloning a
branch to review it would run their command with your shell, your SSH keys and
your tokens.

So `gitscale hook install` takes an allowlist, and bakes it into the hook it
writes:

```
gitscale hook install --global --allow 'github.com/acme/*,git.internal.example/*'
```

Comma-separated glob patterns, where `*` stands for any run of characters and
`?` for exactly one. They are matched, case-insensitively, against the
`host/owner/repo` of the workspace's `origin` — so the SSH and HTTPS spellings
of one repository are the same pattern:

| Pattern | Allows |
|---------|--------|
| `github.com/acme/gitscale` | that one repository |
| `github.com/acme/*` | every repository under that owner, subgroups included |
| `github.com/*` | every repository on that host |
| `*/acme/*` | that owner on any host |
| `*` | everything (the pre-0.3 behaviour) |
| `/srv/workspaces/*` | matched against the workspace path, for a workspace with no remote |

`*` deliberately crosses `/`, so `gitlab.com/acme/*` covers a nested subgroup.
The flip side is that a pattern must be ended deliberately: `github.com/acme/*`
does not match `github.com/acme-evil/x`, but `github.com/acme*` does.

There is no config file. The patterns live in the hook script itself, in
`~/.config/gitscale/hooks/` or `/etc/gitscale/hooks/`, which the shim passes to
gitscale in `GITSCALE_HOOK_ALLOW`. That is what makes them trustworthy: no
branch can reach that file, so a repository cannot vouch for itself.

`gitscale hook status` prints the patterns in effect and whether the current
repository is inside them.

Some consequences worth knowing:

- **`--allow` is required for `--global` and `--system`.** There is no default,
  because guessing one is the bug this exists to prevent. Re-installing without
  `--allow` keeps whatever the previous hook allowed, so upgrading does not
  silently widen or narrow anything.
- **`--local` allows its own repository** without being asked. Installing into
  one repo's `.git/hooks` is already a decision about that repo, and git never
  transfers `.git/hooks`, so the file cannot reach anyone else.
- **A hook installed by gitscale before 0.3 passes no allowlist**, and gitscale
  refuses to run rather than assuming the old behaviour. Re-run
  `gitscale hook install` to choose.
- **A `gitscale pull` or `sync` you type yourself is not restricted.** You chose
  the directory and the moment; the allowlist belongs to the hook that fires
  without being asked. If you clone an untrusted branch and then run `gitscale
  pull` in it by hand, its `post_sync` will run — the checks in the next section
  are what still apply there.

### Values gitscale refuses to pass to git

A `.gitscale.toml` from an untrusted branch can also try to reach git's own
command execution, which no hook allowlist would cover. These are rejected when
the config is loaded, by every command:

- **Remote helper URLs** — `url = "ext::sh -c '…'"` makes git run the rest of
  the URL as a command. Any `helper::` prefix is refused.
- **Option-shaped values** — a `url` or `revision` starting with `-` reaches
  git as a flag, and flags like `--upload-pack=` name a program to run.
- **Directories that escape the workspace** — a repo directory must be relative
  and free of `..`, so a config cannot decide to check out over `~/.ssh`.

## Git hooks

`gitscale hook install` registers gitscale with git so that `gitscale pull` runs
whenever a checkout or merge changes what is in the working tree — a fresh
clone materialises its sub-repositories without anyone remembering to run
anything.

### Scopes

| Scope | Writes | Use for |
|-------|--------|---------|
| `--local` (default) | a hook file in this repo's hooks directory | one repository; repos where a global install is shadowed |
| `--global` | `core.hooksPath` in `~/.gitconfig` | every repo for the current user |
| `--system` | `core.hooksPath` in `/etc/gitconfig` | build machines, where every user should get it |

`--global` and `--system` require `--allow` — see the
[hook allowlist](#hook-allowlist) — because they fire on clones of repositories
nobody has vetted.

`--local` does **not** survive a fresh clone — git never transfers `.git/hooks`
— so it cannot bootstrap a brand-new checkout. Use `--global` or `--system` for
that, and `--local` for repos that already exist or where a global install does
not apply.

### Which hooks, and what they cover

Only `post-checkout` and `post-merge` are installed. Between them:

| Operation | Covered |
|-----------|---------|
| `clone` | yes (`post-checkout`) |
| `checkout` / `switch` | yes |
| `fetch --depth=1` + `checkout FETCH_HEAD` (how CI checks out) | yes |
| `merge`, `git pull` | yes (`post-merge`) |
| `rebase`, `git pull --rebase` | yes (fires `post-checkout` too) |
| `git reset --hard` | **no** |
| plain `fetch` | not needed — the working tree does not change |

`git reset --hard` fires no working-tree hook at all, so gitscale cannot react
to it. `gitscale status` remains the way to spot drift.

### Coexisting with other hooks

A global `core.hooksPath` *replaces* `.git/hooks` rather than adding to it, so
the installed hook always runs the repository's own hook first and reports its
exit status. If a hook of the same name already exists, install moves it aside
to `<name>.local` and chains to it; `uninstall` puts it back.

The installed hook does nothing at all unless the repository root contains a
`.gitscale.toml`. Repos that never opted in stay silent.

### CI

Hooks cannot bootstrap a checkout on hosted CI: GitLab fetches sources before
any job script runs, and hosted runners keep no global git config between jobs.
On runners you control, `gitscale hook install --system` works. Everywhere else,
run `gitscale pull` as an explicit step after checkout.

### Caveats

- A `--global` or `--system` hook makes `git clone` and `git checkout` run
  gitscale against whatever config the branch carries. The
  [hook allowlist](#hook-allowlist) baked into the hook is what keeps that from
  being an invitation.
- A repository that sets its own `core.hooksPath` (husky, lefthook, pre-commit)
  overrides the global one, so a `--global` install does not run there. This is
  silent — `gitscale hook status` reports it, and `--local` is the fix.
- Install refuses to replace an existing `core.hooksPath` without `--force`,
  since doing so would quietly disable those hooks.
- When a hook-triggered pull fails, gitscale reports it on stderr and records it
  in `.git/gitscale-pull-failed`, which `gitscale hook status` surfaces. Git
  collapses any non-zero hook exit to `1` and attributes it to the checkout, so
  the breadcrumb — not the exit status — is what makes a later failure
  diagnosable.

## Recursive dependencies

When a cloned repo contains its own `.gitscale.toml`, GitScale resolves transitive dependencies automatically. Instead of cloning nested copies, it creates **symlinks** from the child's declared paths to the root-level checkouts.

### Example

```
# Root .gitscale.toml
[repos]
"core" = { url = "git@github.com:org/core.git", revision = "main" }
"sharedlibs" = { url = "git@github.com:org/sharedlibs.git", revision = "main" }
```

If `core/.gitscale.toml` declares:

```toml
[repos]
"libs/shared" = { url = "git@github.com:org/sharedlibs.git", revision = "main" }
```

GitScale matches the URL, skips cloning, and creates a symlink: `core/libs/shared → ../../sharedlibs`.

### Rules

- **Recursive by default.** Disable per repo with `recursive = false`.
- **Root is the source of truth.** All transitive deps must be declared in the root config — if missing, GitScale errors out.
- **Root revision wins.** If root specifies a revision, child declarations are ignored silently.
- **Revision deferral.** If root omits `revision`, the child's revision is adopted. If multiple children disagree, GitScale errors out and asks you to pin one in the root config.
- **Works with artefacts.** Extracted artefact folders are scanned for `.gitscale.toml` too.

## Related tools

GitScale occupies the same space as several multi-repo and vendoring tools. The comparison table includes a few rows of language-specific workspace managers for context rather than direct one-to-one equivalents.

*The requirements we have to cover, grouped:*

**R1 — Share code, configs and conventions of any kind**
* share typescript / rust / python clients and shared libraries
* share deployment configurations, like kustomize overlays, helm charts, and terraform modules (for the practice: deployment configs owned by teams / apps, but deployed by the central engine / release process)
* share development environments settings, like docker-compose and nix-shells
* share conventions and LLM skills

**R2 — Share large prebuilt payloads**
* share development datasets, large snapshots, etc.

**R3 — Partial / permission-scoped sharing**
* be able to import a dependency with less permissive access (eg. custom LLM provider code but no trained model data) to facilitate proactive offensive security approach, in other works be able to share repository code partially

**R4 — Version coherence across everything shared**
* have a simple way of identifying related versions of everything shared and corresponding git revisions / branches

**R5 — Direct and transitive dependency management**
* have a management for direct dependencies, which also exports / share different kinds of artefacts (eg. an app depends on repos exporting rust, python and conventions from 3 different repos)
* have a management for transitive dependencies, repo A depends on repo B, which depends on repo C, and so on and repos export / share different kinds of artefacts
* deduplicate shared transitive dependencies — if repo A and repo B both depend on repo C, C is checked out once, not copied per parent
* catch version mismatches instead of silently ending up with several revisions of the same dependency in one checkout

**R6 — Switch between SDK-only and full-stack imports**
* be able switch easily (or completely transparently) between importing SDKs only (eg. pure FE development without having local BE changed and deployed) or importing full stack (eg. full stack development with local BE changes and deployments)

**R7 — Fast, automatic, unsurprising DX**
* the process of importing a repository need to be fast and have good DX (eg. submodules copy entire codebase at imported revision)
* should work completely automatically (i.e not require --include-submodules or --recursive flags alike on checkout)
* should have great DX, default choices are super easy to understand and use, status is clear, automation is an addon but not a replacement for workflows people are used to

**R8 — Work with git worktrees, and reuse what is already on disk**
* a second worktree of the workspace should come up populated, without a manual per-dependency step after `git worktree add`
* checkout time should not scale with the number of worktrees — an existing local copy of a dependency should be reused instead of re-fetched
* the reuse must be safe to undo, since a borrowed copy that is later deleted or garbage-collected breaks the checkouts that borrowed from it

Baseline for every candidate: *should rely and work with git repositories.*

### Coverage

`✔` covered · `~` partial or with caveats · `✘` not covered

| Tool | Config | Approach | R1 any content | R2 large prebuilt | R3 partial share | R4 versions | R5 deps: transitive, dedup, mismatch | R6 SDK ↔ full stack | R7 auto + DX | R8 worktrees + local reuse |
|------|--------|----------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| **GitScale** | single `.gitscale.toml` | separate clones + symlinks | ✔ | ✔ S3 artefacts | ✔ artefact mode | ✔ one pinned config | ✔ recursive, symlink dedup, errors on conflicting revisions | ✔ readonly/artefact ↔ readwrite | ✔ git hooks, shallow, one status | ✔ auto-populated by hook, `--reference` reuse, opt-in `dissociate` |
| git submodules | `.gitmodules` + gitlink | separate clones | ✔ | ✘ | ✘ whole repo only | ✔ pinned SHA | `~` recursive, but nested copies and silent divergence | ✘ | ✘ needs `--recursive` | `~` `--reference` propagates, but `worktree add` leaves empty dirs |
| git subtree | none (in-tree) | merged into main tree | ✔ | ✘ | `~` per-prefix | `~` SHA buried in merges | ✘ | ✘ | `~` in-tree, awkward updates | ✔ in-tree — nothing extra to clone |
| [git-subrepo](https://github.com/ingydotnet/git-subrepo) | `.gitrepo` per subdir | merged into main tree | ✔ | ✘ | `~` per-subdir | ✔ `.gitrepo` records commit | ✘ | ✘ | `~` in-tree, extra binary | ✔ in-tree — nothing extra to clone |
| [Google repo](https://gerrit.googlesource.com/git-repo) | `manifest.xml` | separate clones | ✔ | ✘ | ✘ | ✔ manifest snapshot | ✘ flat manifest, no transitive | ✘ | ✘ manual `repo sync` | ✔ `--reference`, `--dissociate`, `--worktree` |
| [vcstool](https://github.com/dirk-thomas/vcstool) | `.repos` YAML | separate clones | ✔ | ✘ | ✘ | ✔ `.repos` pins | ✘ flat list, no transitive | ✘ | `~` manual `vcs import` | `~` `--shallow` only, no local reuse |
| [west](https://github.com/zephyrproject-rtos/west) | `west.yml` | separate clones | `~` mostly | ✘ | ✘ | ✔ `west.yml` | `~` manifest imports, name clashes error, no dedup | ✘ | `~` manual `west update` | `~` `update.auto-cache` reference cache, no worktree mode |
| [myrepos (mr)](https://myrepos.branchable.com/) | `.mrconfig` | separate clones, any VCS | ✔ | ✘ | ✘ | ✘ no pinning | ✘ | ✘ | `~` status only | ✘ |
| [meta](https://github.com/mateodelnorte/meta) | `.meta` JSON | separate clones + plugins | ✔ | ✘ | ✘ | ✘ no pinning | ✘ | ✘ | `~` plugin-dependent | ✘ |
| [gclient](https://chromium.googlesource.com/chromium/tools/depot_tools) | `DEPS` (Python) | separate clones + hooks | `~` mostly | `~` CIPD/GCS via hooks | ✘ | ✔ `DEPS` pins | `~` recursive DEPS, conflicts error, dedup is path-keyed | ✘ | ✘ manual sync, Python config | `~` `--cache-dir` shared clones, no worktree mode |
| Yarn workspaces | `package.json` | JS/TS monorepo workspace | ✘ JS/TS only | `~` registry tarballs | `~` published subset | ✔ lockfile | ✔ hoisting + range/peer checks, package-level | `~` `link` / resolutions | n/a single repo | n/a single repo |
| Cargo workspaces | `Cargo.toml` | Rust multi-crate workspace | ✘ Rust only | ✘ | `~` published crate | ✔ lockfile | ✔ semver unification, not repo dedup | `~` `[patch]` / path override | n/a single repo | n/a single repo |
| Go workspaces | `go.work` | Go multi-module workspace | ✘ Go only | ✘ | `~` published module | ✔ `go.mod` / `go.sum` | ✔ MVS picks one version, not repo dedup | `~` `go.work use` | n/a single repo | n/a single repo |

The three workspace managers work inside a single repo, so R7 and R8 never arise for them — they don't address multi-repo checkout at all.

### GitScale highlights

- **Readonly enforcement.** Vendored dependencies have their write bits stripped on disk, so accidental edits fail loudly instead of drifting silently. Submodules, repo, and vcstool leave everything writable.
- **Artefact mode.** Entries can be pulled as prebuilt `tar.gz` archives from any S3-compatible bucket instead of cloned — useful for large generated outputs or closed-source blobs. No comparable tool ships this; you'd otherwise bolt on a separate artefact/LFS pipeline.
- **Native S3 storage.** Signing and transfer are built in (no `aws` CLI or SDK required); works with AWS, MinIO, R2, B2, Spaces, GCS, or a local directory.
- **Transitive dedup via symlinks.** When two nested configs depend on the same repo, GitScale checks it out once at the root and symlinks the rest, avoiding duplicate clones. Submodules and repo produce independent nested copies.
- **CI-aware shallow cloning.** Automatically shallow-clones everything under `CI=1`, and readonly repos are always shallow — faster, smaller checkouts without extra flags.
- **Worktrees cost almost nothing.** A second worktree of the workspace populates itself (the `post-checkout` hook pulls), and each sub-repository is cloned from the copy the main worktree already has rather than re-fetched — see [Reusing a local copy](#reusing-a-local-copy). Submodules leave a new worktree full of empty directories until you run `git submodule update --init` by hand.
- **CI credentials without pipeline setup.** Inside a GitLab or GitHub job, entries hosted on that same server are fetched over HTTPS with the job token — scoped to that host, with the token never written to `.git/config` or a command line. No `insteadOf` rewriting in `.gitlab-ci.yml`.
- **Rich, single-glance status.** One `status` table (with JSON output) surfaces ahead/behind, detached, ref-mismatch, dirty, stale, and broken-symlink states across every repo.
- **One human-readable config.** A single TOML file versus `.gitmodules` + gitlink entries, XML manifests, or Python `DEPS`.

### Design choices

- **Separate history per repo.** subtree and git-subrepo vendor code *into* your main repo, so the dependency's file history lives directly in the parent repository. That is what earns them R8 for free — a worktree of the parent already contains everything — but the cost is paid once per clone of the parent instead, whose history now carries every dependency's. GitScale keeps sub-repos as separate working trees by design, while the parent repo tracks the selected dependency revision in `.gitscale.toml`.
- **External binary.** Submodules and subtree ship with git and need no extra install; GitScale is a separate binary.
- **Git-only.** myrepos handles Git, Mercurial, Bazaar, SVN, and more. GitScale targets Git (plus its own artefact archives).
- **Simpler workflow model.** Google repo and west help coordinate branch creation, topic work, and release manifests across many repos. GitScale can pin each dependency to a branch, tag, or commit, but it does not try to manage a shared multi-repo branching lifecycle. When `.gitscale.toml` pins child repos to immutable tags or commit SHAs, that config effectively becomes the release manifest: the system version is defined as an assembly of specific subsystem versions.
- **R8 follows Google repo's design.** `repo init` has offered `--reference`, `--dissociate` and a `--worktree` mode for years, and gclient solves the same problem with a shared `--cache-dir` mirror. GitScale's approach is the same idea with the configuration removed: no mirror to set up and no flag to remember, because it works out the source workspace from git itself. The tradeoff is less control — repo lets you point at an arbitrary mirror, GitScale only reuses a workspace this one was actually derived from.
- **Self-contained tool.** meta has a plugin system and repo/gclient have larger surrounding ecosystems, while GitScale keeps the core workflow built into one tool. That simplifies operation and keeps behavior directly controllable for Partly's needs.
- **Linux-first symlink dedup.** Symlink-based dedup is efficient and natural on Linux, which is the primary development environment for Partly engineers. The tradeoff is that it is more awkward on Windows without developer mode or elevated privileges.

### When to pick GitScale

Choose GitScale when you want submodule-style separate checkouts but with a friendlier single config, enforced-readonly vendoring, prebuilt artefact delivery, and automatic deduplication of shared transitive dependencies. Prefer subtree/git-subrepo if you need everything in one repo and one history, or Google repo/west if you're managing a very large manifest-driven project with heavy multi-branch workflows.

## License

MIT

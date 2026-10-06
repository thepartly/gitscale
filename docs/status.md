# 2.2 The workspace: `git scale ls` and `git explain`

- [Reading the table](#reading-the-table)
- [Columns](#columns)
- [Status flags](#status-flags)
- [The RESOLUTION column](#the-resolution-column)
- [Topics](#topics)
- [Warnings](#warnings)
- [Icons and colour](#icons-and-colour)
- [Why a checkout has its revision: `git explain`](#why-a-checkout-has-its-revision-git-explain)
- [Fetching first](#fetching-first)
- [JSON output](#json-output)
- [What `ls` does not cover](#what-ls-does-not-cover)

`git scale ls` — or `git scale list` — is the one command that answers "what
state is this workspace in" for every checkout at once — the root's entries and
the [implicit dependencies](recursive-dependencies.md#implicit-dependencies)
its repositories bring in. It is read-only. It resolves from what is on this
machine — in CI, what the job's placement fetched — unless `--fetch` asks the
remotes first; even then it changes no checkout: it updates the root's [stores](stores.md), and for
artefacts the record of what the registry has and the small config layer
resolution reads.

```
git scale ls                    # table
git scale ls --fetch            # fetch remote state first, then report
git scale ls --format json      # machine-readable
git explain                     # how each shared checkout got its revision
```

## Reading the table

```
    REPO             PATH   AS         REF       EXPECTED   STATUS
⇓   apps/api         -      source     feat/x    feat/x     -3
!   apps/web         -      source     feat/x    feat/x     dirty
✔   imports/core     -      source     8c1d0e2   main       ok
✘   imports/new      -      source     —         main       missed
≠   imports/proto    -      source     41be7a0   main       ref-mismatch
⤷   imports/shared   ../s   source     8c1d0e2   main       symlink
✔   meta/frontend    -      artefact   v2.1.0    v2.1.0     ok
⊘   core/imports/x   -      -          —                    orphan
```

When some checkout's revision came about through
[resolution](recursive-dependencies.md#how-a-revision-is-chosen) rather than
straight from the root, a `RESOLUTION` column follows — see
[below](#the-resolution-column).

One row per declared entry, sorted by directory, then one per implicit checkout,
followed by a row for each
[orphaned symlink](recursive-dependencies.md#orphaned-symlinks) found.

## Columns

| Column | Meaning |
|---|---|
| *(unnamed)* | Status icon — see [icons and colour](#icons-and-colour) |
| `REPO` | The directory the entry declares. For an orphan row, the path of the leftover symlink |
| `PATH` | Where a symlinked entry points; `-` for an ordinary checkout. Only [recursive dependencies](recursive-dependencies.md) deduped into one checkout are symlinks |
| `AS` | How the checkout arrives: `source` or `artefact` — see [artefacts](artefacts.md#choosing-how-a-checkout-arrives). `source → artefact` while the next placement is still to apply a preference |
| `REF` | The ref currently checked out: the topic branch, or an abbreviated commit when HEAD is detached. For an artefact, the release it was installed from. `—` when there is no checkout. For a symlink row, the ref of the checkout it points at |
| `EXPECTED` | The revision [resolution](recursive-dependencies.md#how-a-revision-is-chosen) chose — the root's own, a higher one a dependency asked for, or the topic branch. A SHA is abbreviated to 7 characters, as `REF` spells a detached HEAD in most repositories (git may use more digits in a very large one) |
| `STATUS` | The flags below, comma-separated, or `ok` |
| `RESOLUTION` | How the revision was chosen, when there is more to it than the root asking for it. Shown only when some row has something to say |

## Status flags

`missed` and `symlink` are reported alone. Everything else combines, in this
order.

| Flag | Meaning |
|---|---|
| `ok` | Clean, and on the expected revision |
| `missed` | No checkout yet: the directory does not exist, or holds no repository — the next [placement](workflow.md#placement) checks it out |
| `symlink` | Resolved as a symlink to a root-level checkout (a deduped [recursive dependency](recursive-dependencies.md)) |
| `unlinked` | A dependency inside this repo that should be a symlink is a real clone instead — see [unlinked checkouts](recursive-dependencies.md#unlinked-checkouts). The next [placement](workflow.md#placement) relinks it |
| `unlinked, modified` | …and that clone has uncommitted changes or unpushed commits, so relinking would lose work. `git scale sync --force` overrides |
| `unlinked, dirty` | …and the repo itself has uncommitted changes |
| `untracked-links` | Git sees the dependency links GitScale planted here as untracked files, though placement keeps them in `info/exclude` — see below. Not `dirty`: they are not anyone's work |
| `dirty` | The working tree has uncommitted changes (`git status --porcelain` is non-empty, untracked files included — except the links GitScale planted, which are `untracked-links`) |
| `foreign` | Not a worktree of the root's store: a clone made by hand or by an older GitScale. Placement leaves it alone — see [stores](stores.md#checkouts-gitscale-did-not-make) |
| `stale` | A CI checkout whose commit differs from upstream. A depth-1 checkout cannot produce an exact behind count, so this stands in for it |
| `+N` | On a topic branch, N commits ahead of its upstream |
| `-N` | On a topic branch, N commits behind its upstream |
| `ref-mismatch` | The checkout is not where resolution puts it: detached at another commit, or not on the topic branch. For an artefact: installed for another release than the one wanted now |
| `missing` | Artefact only: the release wanted had no image in the registry when last fetched |
| `changed` | Artefact only: the installed release's image was re-published with different files; the next [placement](workflow.md#placement) installs it |
| `orphan` | A leftover GitScale symlink whose dependency is no longer declared; its target still resolves |
| `orphan, broken` | …and its target no longer exists |
| `override` | An override holds the checkout below a request it beat; `RESOLUTION` says which |
| `unresolved` | Resolution could not settle this checkout; `RESOLUTION` says why |

The symlinks GitScale plants for a repository's own dependencies are not its
owner's work, so they never make it `dirty`, and placement keeps them out of
git's sight in the repository's `info/exclude`. Should git see them anyway —
that block removed, or a `.gitignore` of the repository's own taking a path
back in — the row says `untracked-links`, and a line under the table says how
to put them back:

```
    REPO        PATH   AS         REF       EXPECTED   STATUS
!   imports/b   -      source     1498d55   v2.0.0     untracked-links
hint: imports/b: git sees the dependency links GitScale planted there (libs/d); git scale sync puts them back in its info/exclude
```

See [ignoring the checkout directory](dependencies.md#ignoring-the-checkout-directory).

`ref-mismatch` compares where HEAD actually is, not the text of the two columns.
A checkout off the topic is detached, so `REF` reads as a commit and can never
equal a tag or branch name — that alone is not a mismatch. Detached *at the
wrong commit* is.

A checkout taken as an artefact has no working tree to be dirty and no history to count, so
its row only ever carries `ok`, `missed`, `symlink`, `unlinked`, the artefact
flags, or resolution's `override` and `unresolved`. They compare what is installed with what the last
`git scale fetch` — or `git scale ls --fetch` — saw; see
[artefacts → status](artefacts.md#status).

## The RESOLUTION column

`STATUS` is the state of the checkout; `RESOLUTION` is where its revision came
from. It appears when some row has something to say, and names the winner
only — with a count when more than one repository asked:

```
    REPO         PATH   AS         REF       EXPECTED   STATUS       RESOLUTION
✔   imports/d    -      source     3f2a9c1   v1.5.0     ok           raised from v1.2.0 by imports/c, 3 requests
↧   imports/f    -      source     9e01c44   v1.4.0     override     held at v1.4.0, imports/c wants v1.6.0, 2 requests
?   imports/h    -      source     —         main       unresolved   not fetched yet: run git scale ls --fetch
✔   imports/sh   -      source     77c0a1d   main       ok           implicit via imports/d
hint: git explain <dir> lists every request behind a revision
```

| Text | Meaning |
|---|---|
| `raised from v1.2.0 by imports/c` | The root asked for less; a dependency asked for more, and won |
| `held at v1.4.0, imports/c wants v1.6.0` | The root's override holds it below a request it beat (`override` in STATUS) |
| `held at v1.4.0 by imports/b, imports/e wants v1.6.0` | The same, for an override in a dependency |
| `…, imports/b wants develop (no such revision)` | The override replaced a request for a revision the repository does not have |
| `behind imports/c's v1-2026.09.30` | Won by position, but its commit is behind what that request asked for. Only where history is on local disk; the icon is `↧` when the row is otherwise `ok` |
| `implicit via imports/b` | Not in the root config; brought in by the repository named |
| `2 majors` | This repository has a checkout per major |
| `3 requests` | That many repositories asked for it. `git explain` lists them |
| `not fetched yet: run git scale ls --fetch` | Resolution could not settle this checkout from what is on this machine (`unresolved` in STATUS) |
| `its dependencies are not fetched yet: run git scale ls --fetch` | Its own revision is known, but its `.gitscale.toml` at that commit is not on this machine (`unresolved` in STATUS) |
| `see the error above` | Resolution failed as a whole; the error is printed on stderr, as `error: …` |
| `pinned by imports/b` | Held at its pin on a topic, because that repository pins the topic's branch — see [topics](topics.md#inside-a-topic) |

When any row counts more than one request, a `hint:` line under the table
points at [`git explain`](#why-a-checkout-has-its-revision-git-explain), which lists them
all. A checkout with nothing on disk yet is only `missed`.

## Topics

On a [topic](topics.md), the table starts with the topic's branch and what may
merge next, and each topic row's `RESOLUTION` says how it joined, where its
change stands and what it waits on:

```
topic feat/price-cache · next to merge: imports/d
    REPO        PATH   AS         REF                EXPECTED           STATUS   RESOLUTION
✔   imports/b   -      source     feat/price-cache   feat/price-cache   ok       topic, waits on imports/d, no tag
✔   imports/d   -      source     feat/price-cache   feat/price-cache   ok       topic, no tag, implicit via imports/b
```

| Text | Meaning |
|---|---|
| `topic` | On the topic branch in its store: joined here or in another root worktree |
| `topic, from remote` | Its remote has the topic branch; the next placement puts it on a local branch tracking that |
| `waits on imports/d` | Asks, directly or further down, for a topic slot that is not promoted yet |
| `behind v1-2026.09.30 wanted by imports/c: rebase it` | The topic branch lacks a release another repository now asks for |
| `no change yet`, `no tag`, `promoted → <tag>`, … | Where its change stands — see [promotion](topics.md#promotion-git-upgrade) |

The promotion states and `behind` need history, so they are worked out from
the root's stores, and never in CI.

## Warnings

What is wrong with the workspace as a whole, rather than with one row, is
printed as a `warning:` line under the table. A root with no fetch refspec — a
`git clone --bare` layout set up without one — never updates `origin/*`:

```
warning: the root repository has no fetch refspec, so origin/* is never updated
  ahead/behind and upstreams in this table may be wrong. To fix:
  git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*' && git fetch
```

## Icons and colour

The first column always holds an icon. Colour follows the [rule every command
uses](cli.md#colour): on a terminal only, so piped output and CI logs get none;
`--color always` forces it, `--color never` or `NO_COLOR` turns it off.

The first matching rule wins, so a row with several flags takes the icon of the
most serious one.

| Icon | Colour | Shown for |
|---|---|---|
| `✔` | green | `ok` |
| `⤷` | cyan | `symlink` |
| `✘` | red | `missed` |
| `⊘` | yellow / bright red | `orphan` / `orphan, broken` |
| `~` | yellow / bright red | `unlinked` / `unlinked` with anything else |
| `!` | bright red / yellow | `dirty`, `foreign`, `missing`, `changed` / `untracked-links` on its own |
| `≠` | bright red | `stale`, `ref-mismatch` |
| `?` | yellow | `unresolved` |
| `↧` | yellow | `override`; or a winner `behind` a request (see RESOLUTION) on a row otherwise `ok` |
| `⇅` | yellow | ahead and behind |
| `⇑` | yellow | ahead only |
| `⇓` | yellow | behind only |

The `REF` cell is also painted yellow on a `ref-mismatch`, so the wrong ref is
visible without reading the last column.

## Why a checkout has its revision: `git explain`

```
$ git explain imports/d
imports/d  git@github.com:org/d.git  major 1  source
  selected  v1.5.0 (3f2a9c1)  tag, highest semver
  requests
    root                     v1.2.0  tag
    root → imports/b@v2.1.0  v1.3.1  tag
    root → imports/c@v0.9.0  v1.5.0  tag   selected
```

Every request for the checkout: the path it came by, the revision and what it
names (`branch`, `tag` or `sha`), and which one won and why — `highest
semver`, `highest calendar version`, `asked for by a repository above the
other`, `same commit`, `override`, `only request`. `override`, `no such revision`,
`overruled by` and `selected` mark the requests they apply to.

Name directories to see those — paths from the current directory, `.` inside
a checkout; see [directory arguments](cli.md#directory-arguments). `git
explain` alone shows every checkout more than one repository asks for. Like
`ls`, it resolves from what is on this machine unless `--fetch` is given.

## Fetching first

Without `--fetch`, `ls` reports what is already on disk: ahead/behind counts
come from the remote-tracking refs as they stand, which may be old.

`--fetch` updates them first: one fetch per [store](stores.md), however many
checkouts and root worktrees use it. Artefact entries resolve their revision with `ls-remote` and ask
the registry whether that commit has an image, and record both. A fetch that
fails is reported on stderr as
`fetch <directory>: <reason> (showing the last fetched state)`, and `ls` still
reports, on whatever it has.

Add `-v` to see which repository is being fetched.

## JSON output

`--format json` prints an array. One object per checkout, declared or implicit:

```json
{
  "directory": "imports/core",
  "exists": true,
  "current_ref": "8c1d0e2",
  "expected_ref": "main",
  "clean": true,
  "detached": true,
  "ahead": 0,
  "behind": 0,
  "stale": false,
  "symlink": false,
  "symlink_target": "",
  "untracked_links": [],
  "foreign": false,
  "source_hash": "0d7d3387c313ae68b5266627b8516c367b79da27c00654dd10dacce856d5c847"
}
```

How resolution got there is in the same object, as structure only:

```json
{
  "declared_ref": "v1.2.0",
  "resolved_ref": "v1.5.0",
  "resolved_commit": "3f2a9c1e5b7d4e8a9c217d4e5f6a8b90c1d2e3f4",
  "revision_kind": "tag",
  "class": "1",
  "as": "source",
  "as_reason": "default",
  "as_next": null,
  "implicit": false,
  "resolution": "raised",
  "reason": "semver",
  "unresolved": null,
  "unread": null,
  "notes": ["raised from v1.2.0 by imports/c", "2 requests"],
  "requests": [
    { "from": "root", "chain": [], "directory": "imports/d", "revision": "v1.2.0",
      "revision_kind": "tag", "override": false, "selected": false,
      "overruled_by": null, "ahead": false, "missing": false },
    { "from": "imports/c", "chain": ["imports/c@v0.9.0"], "directory": "vendor/d",
      "revision": "v1.5.0", "revision_kind": "tag", "override": false,
      "selected": true, "overruled_by": null, "ahead": false, "missing": false }
  ]
}
```

| Field | Values |
|---|---|
| `expected_ref`, `resolved_ref` | The revision resolution chose; empty when nobody asks for one |
| `declared_ref` | The root's own revision; empty for an implicit checkout or an entry without one |
| `revision_kind` | `branch`, `tag`, `sha` or `build` — consumers decide for themselves which they treat as pinned. A checkout nobody gives a revision follows a branch |
| `class` | The major: `1`, `0.4`, or `any` |
| `as` | How the checkout arrives on disk now: `source` or `artefact` |
| `as_reason` | Why that form: `topic` (on the topic), `no-access` (its sources cannot be read), `preferred` or `default` |
| `as_next` | The form the next placement gives, when it differs; else `null` |
| `resolution` | `only`, `root` (the root's own request won), `raised`, `highest` (the root asked for nothing), `override`, `follow` (no revision asked for) or `unresolved` |
| `reason` | `only`, `semver`, `calver`, `position`, `equal` or `override` |
| `unread` | Why the checkout's own dependencies could not be read, when they could not |
| `untracked_links` | The planted links git sees as untracked files, relative to the checkout |
| `source_hash` | What its own pipeline builds — see [`git scale hash`](cli.md#git-scale-hash); `null` with uncommitted changes |
| `topic` | `null` off the topic; else `branch`, `commit`, `developed` (on a local branch, rather than only the remote's), `pin` (`revision`, `commit`, `by`: what a merge would ship) and `state` (`unchanged`, `no-tag`, `no-release`, `no-image`, `promoted`, `cannot-tell`, `held`, `unknown`) |
| `pinned_by` | The repository whose pinned branch holds this slot at its pin, or `null` |

An artefact's object also carries what is installed and what the last
fetch saw, with the flags those produce:

```json
"artefact": {
  "installed": { "tag": "v2.0.0", "digest": "sha256:77d0…" },
  "remote":    { "tag": "v2.1.0", "digest": null },
  "flags": ["ref-mismatch", "missing"]
}
```

`remote` is `null` until a fetch has run for the release wanted, and its
`digest` is `null` when that release has no image.

Orphaned symlinks appear as their own objects:

```json
{ "directory": "core/imports/x", "orphan": true, "broken": false }
```

On a topic, an object names it and what may merge next; workspace warnings come
last:

```json
{ "topic": "feat/price-cache", "next_to_merge": ["imports/d"] }
{ "warnings": ["the root repository has no fetch refspec, …"] }
```

## What `ls` does not cover

- **The workspace repository itself** is not a row in the table. Its own state
  is ordinary `git status` territory; `git scale status` runs it in the root
  and every checkout on the topic.
- **The CI cache** has its own command — [`git scale cache status`](stores.md#cache-status).
- **Nested repositories GitScale does not manage** — a clone somebody made by
  hand inside a checkout — are invisible to `ls`.

---

[← 2.1 Declaring dependencies](dependencies.md) · [Contents](README.md) · [Next → 2.3 Recursive dependencies](recursive-dependencies.md)

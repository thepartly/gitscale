# 2.2 Status

- [Reading the table](#reading-the-table)
- [Columns](#columns)
- [Status flags](#status-flags)
- [Icons and colour](#icons-and-colour)
- [Fetching first](#fetching-first)
- [JSON output](#json-output)
- [What status does not cover](#what-status-does-not-cover)

`gitscale status` is the one command that answers "what state is this workspace
in" for every declared repository at once. It is read-only: it never fetches,
clones or modifies anything unless you pass `--fetch`, and even then it only
updates remote-tracking refs.

```
gitscale status                 # table
gitscale status --fetch         # fetch remote state first, then report
gitscale status --format json   # machine-readable
```

## Reading the table

```
      REPO             PATH   MODE        REF       EXPECTED   STATUS
✔     imports/core     -      readonly    main      main       ok
✔     imports/utils    -      readwrite   v2.1.0    v2.1.0     ok
⤷     imports/shared   ../s   readonly    main      main       symlink
!     apps/web         -      readwrite   main      main       dirty
≠     imports/proto    -      readonly    develop   main       ref-mismatch
⇓     apps/api         -      readwrite   main      main       -3
✘     imports/new      -      readwrite   —         main       missed
⊘     core/imports/x   -      -           —                    orphan
```

One row per declared entry, in config order, followed by a row for each
[orphaned symlink](recursive-dependencies.md#orphaned-symlinks) found.

## Columns

| Column | Meaning |
|---|---|
| *(unnamed)* | Status icon — see [icons and colour](#icons-and-colour) |
| `REPO` | The directory the entry declares. For an orphan row, the path of the leftover symlink |
| `PATH` | Where a symlinked entry points; `-` for an ordinary checkout. Only [recursive dependencies](recursive-dependencies.md) deduped into one checkout are symlinks |
| `MODE` | `readwrite`, `readonly` or `artefact` |
| `REF` | The ref currently checked out: a branch name, or an abbreviated commit when HEAD is detached. `artefact` for an extracted artefact, `—` when the directory does not exist. For a symlink row, the ref of the checkout it points at |
| `EXPECTED` | The `revision` from `.gitscale.toml`. A SHA is abbreviated to 7 characters so it lines up with `REF` |
| `STATUS` | The flags below, comma-separated, or `ok` |

## Status flags

`missed` and `symlink` are reported alone. Everything else combines, in this
order.

| Flag | Meaning |
|---|---|
| `ok` | Clean, and on the expected revision |
| `missed` | The directory does not exist yet — run [`clone`](workflow.md#clone) or [`pull`](workflow.md#pull) |
| `symlink` | Resolved as a symlink to a root-level checkout (a deduped [recursive dependency](recursive-dependencies.md)) |
| `unlinked` | A dependency inside this repo that should be a symlink is a real clone instead. [`sync`](workflow.md#sync) relinks it |
| `unlinked, modified` | …and that clone has uncommitted changes or unpushed commits, so relinking would lose work. `sync --force` overrides |
| `unlinked, dirty` | …and the repo itself has uncommitted changes |
| `dirty` | The working tree has uncommitted changes (`git status --porcelain` is non-empty, untracked files included) |
| `cache-broken` | The checkout borrows objects from a cache entry that is gone. It cannot read its own history — run [`gitscale cache repair`](caching.md#cache-repair) |
| `stale` | Shallow clone whose commit differs from upstream. A shallow repo cannot produce an exact behind count, so this stands in for it |
| `detached` | HEAD is detached. Normal for a tag- or SHA-pinned entry |
| `+N` | N commits ahead of upstream |
| `-N` | N commits behind upstream. For an artefact entry, `-1` means the remote archive's ETag differs from the extracted one |
| `ref-mismatch` | The checkout is not on the declared revision |
| `orphan` | A leftover GitScale symlink whose dependency is no longer declared; its target still resolves |
| `orphan, broken` | …and its target no longer exists |

A repository that declares its own dependencies reads as `dirty` unless it
ignores its import directory: the symlinks GitScale plants there are untracked
files to git. See
[ignoring the checkout directory](dependencies.md#ignoring-the-checkout-directory).

`ref-mismatch` compares where HEAD actually is, not the text of the two columns.
A tag or SHA leaves HEAD detached, so `REF` reads as a commit and can never
equal a tag name — that alone is not a mismatch. Detached *at the wrong commit*
is.

## Icons and colour

The first matching rule wins, so a row with several flags takes the icon of the
most serious one.

| Icon | Colour | Shown for |
|---|---|---|
| `✔` | green | `ok` |
| `⤷` | cyan | `symlink` |
| `✘` | red | `missed` |
| `⊘` | yellow / bright red | `orphan` / `orphan, broken` |
| `~` | yellow / bright red | `unlinked` / `unlinked` with anything else |
| `!` | bright red | `dirty`, `cache-broken` |
| `≠` | bright red | `stale`, `ref-mismatch` |
| `⇅` | yellow | ahead and behind |
| `⇑` | yellow | ahead only |
| `⇓` | yellow | behind only |
| `◆` | cyan | `detached` on its own |

The `REF` cell is also painted yellow on a `ref-mismatch`, so the wrong ref is
visible without reading the last column.

## Fetching first

Without `--fetch`, status reports what is already on disk: ahead/behind counts
come from the remote-tracking refs as they stand, which may be old.

`--fetch` updates them first — through the [object cache](caching.md) like every
other network operation, so it costs one fetch per repository per *machine*, not
per workspace. Artefact entries get a HEAD request that refreshes
`.etag-remote`. Failures during the fetch are ignored: status still reports, on
whatever it has.

Add `-v` to see which repository is being fetched.

## JSON output

`--format json` prints an array. One object per declared entry:

```json
{
  "directory": "imports/core",
  "exists": true,
  "current_ref": "main",
  "expected_ref": "main",
  "clean": true,
  "detached": false,
  "ahead": 0,
  "behind": 0,
  "mode": "readonly",
  "stale": false,
  "symlink": false,
  "symlink_target": "",
  "cache": "mirror"
}
```

`cache` says what the [object cache](caching.md) is doing for that checkout:

| Value | Meaning |
|---|---|
| `-` | Nothing cached for this repository, and the checkout owns its objects |
| `mirror` | Borrowing from a mirror entry |
| `snapshot` | Built from a snapshot entry that holds this exact commit — CI's flow |
| `workspace` | Borrowing from a [source workspace](caching.md#where-objects-come-from) instead |
| `copy` | The checkout owns its objects, but an entry exists for the repository anyway |
| `broken` | Borrowing from something that is no longer there |

Orphaned symlinks appear as their own objects:

```json
{ "directory": "core/imports/x", "orphan": true, "broken": false }
```

The last element is always a summary of the cache itself:

```json
{ "cache_dir": "/home/dev/.local/share/gitscale", "entries": 4, "bytes": 107315 }
```

or `{ "cache_dir": null }` when the cache is off (`--no-cache`, or
`[cache] enabled = false`).

## What status does not cover

- **The workspace repository itself** is not a row in the table. Its own state
  is ordinary `git status` territory.
- **The cache** has its own command — [`gitscale cache status`](caching.md#cache-status).
  `gitscale status` stays about the workspace, with one exception: a checkout
  borrowing from a deleted entry is flagged `cache-broken`, because nothing else
  would tell you.
- **Nested repositories GitScale does not manage** — a clone somebody made by
  hand inside a checkout — are invisible to status.

---

[← 2.1 Declaring dependencies](dependencies.md) · [Contents](README.md) · [Next → 2.3 Recursive dependencies](recursive-dependencies.md)

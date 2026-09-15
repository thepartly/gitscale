# 2.3 Recursive dependencies

- [The idea](#the-idea)
- [Worked example](#worked-example)
- [Hoisting: the root is the source of truth](#hoisting-the-root-is-the-source-of-truth)
- [Deduplication by symlink](#deduplication-by-symlink)
- [Revision resolution and the mismatch check](#revision-resolution-and-the-mismatch-check)
- [Turning it off: `recursive = false`](#turning-it-off-recursive--false)
- [Unlinked clones](#unlinked-clones)
- [Orphaned symlinks](#orphaned-symlinks)
- [When resolution runs](#when-resolution-runs)

## The idea

A repository in the workspace can carry its own `.gitscale.toml`, declaring its
own dependencies. GitScale reads those configs and resolves them — but it never
creates a nested copy. Every repository in the workspace is checked out **once**,
at the path the root config gives it, and a dependant that wants it at some
other path gets a symlink.

Two things follow, and they are the point of the feature:

- **Hoisting.** All transitive dependencies live at the root, so there is one
  place that says which revision of anything is in play.
- **Version coherence.** Two dependants cannot silently end up with two
  different revisions of the same repository, because there is only one
  checkout. Disagreement is an error, not a duplicate.

## Worked example

Root `.gitscale.toml`:

```toml
[repos]
"core"       = { url = "git@github.com:org/core.git",       revision = "main" }
"web"        = { url = "git@github.com:org/web.git",        revision = "main" }
"sharedlibs" = { url = "git@github.com:org/sharedlibs.git", revision = "main" }
```

`core/.gitscale.toml`, committed in the `core` repository:

```toml
[repos]
"imports/shared" = { url = "git@github.com:org/sharedlibs.git", revision = "main" }
```

`web/.gitscale.toml`:

```toml
[repos]
"vendor/shared" = { url = "git@github.com:org/sharedlibs.git", revision = "main" }
```

After `gitscale clone`:

```
workspace/
  .gitscale.toml
  core/
    .gitscale.toml
    imports/shared  → ../../sharedlibs   (symlink)
  web/
    .gitscale.toml
    vendor/shared   → ../../sharedlibs   (symlink)
  sharedlibs/                            (the only checkout)
```

Each link is created relative to its own directory, so the workspace can be
moved or copied without breaking them. GitScale prints each one as it goes:

```
  link  core/imports/shared → sharedlibs
  link  web/vendor/shared → sharedlibs
```

## Hoisting: the root is the source of truth

Matching is by **URL**, normalised so that transport does not matter:
`git@github.com:org/repo.git`, `https://github.com/ORG/repo` and
`https://github.com/org/repo.git` are all the same repository.

If a child declares a dependency the root does not, resolution fails:

```
Error: repo 'core' requires 'imports/shared' (url: git@github.com:org/sharedlibs.git)
but it is not declared in the root .gitscale.toml
```

The fix is to add it to the root config — `gitscale add sharedlibs <url> <rev>`
— which is also the moment someone chooses the revision the whole workspace
will use.

Transitive depth is unlimited, and falls out of the same rule: a
grandchild dependency has to be declared at the root too, which makes it a
top-level entry, whose own config GitScale reads in turn. The graph is always
resolved flat.

## Deduplication by symlink

A symlink is created when the target checkout exists; entries whose target is
not there yet are simply skipped and picked up on a later run. GitScale never
clobbers a real file or directory sitting at a link path — that situation is
reported by [`status`](status.md) as `unlinked` and fixed by
[`sync`](workflow.md#sync).

A few consequences worth knowing:

- Symlinked entries are skipped by `push`, `commit` and [`clean`](clean.md) —
  the real checkout is the one that gets committed and cleaned.
- `status` shows them as `⤷ symlink`, with the link target in the `PATH` column
  and the ref of the checkout it points at in `REF`.
- Extracted [artefact](dependencies.md#artefact) directories are scanned for a
  nested `.gitscale.toml` as well, so a prebuilt payload can declare
  dependencies too.
- Symlink dedup is a Unix mechanism. On Windows it needs developer mode or
  elevated privileges.

## Revision resolution and the mismatch check

Which revision wins depends on whether the root has an opinion:

| Root `revision` | Child `revision` | Result |
|---|---|---|
| set | anything | **The root wins**, silently. The child's declaration is ignored |
| omitted | set by one child | The child's revision is **adopted** and checked out |
| omitted | set by two children, the same | Adopted |
| omitted | set by two children, different | **Error** — see below |
| omitted | omitted | The remote's default branch |

Deferring the revision — leaving it out of the root config — lets the
repositories that actually depend on something say what they need, while the
root still guarantees a single checkout. When they disagree, GitScale refuses
rather than picking one:

```
Error: conflicting revisions for 'git@github.com:org/sharedlibs.git':
'core' wants 'v1.2.0', 'web' wants 'v2.0.0'. Pin a revision in the root
.gitscale.toml to resolve.
```

This is the version-mismatch check: you get an error at resolution time instead
of a workspace that builds against one revision and ships another.

An adopted revision is checked out during [`clone`](workflow.md#clone) (and so
during `sync`, which starts with a clone). [`pull`](workflow.md#pull) re-creates
the symlinks but does not re-checkout an adopted revision — a pull is not the
place to move a checkout somebody may be working in.

## Turning it off: `recursive = false`

```toml
"vendor/tools" = { url = "https://github.com/org/tools.git", revision = "main", recursive = false }
```

GitScale then does not read that repository's `.gitscale.toml` at all. It
creates no symlinks inside it, validates none of its dependencies — and
[`clean`](clean.md) skips it entirely, since its keep-list is out of reach and
cleaning a repo without knowing what it wants kept is worse than leaving it
alone.

Use it for a repository that happens to carry a `.gitscale.toml` of its own that
this workspace has no business acting on.

## Unlinked clones

If a path that should be a symlink holds a real clone instead — someone cloned
into it by hand, or it predates the dependency being hoisted — `status` flags
the **parent repo** as `unlinked`, and `sync` fixes it:

- A clean clone is removed and the symlink restored automatically.
- A clone with uncommitted changes or unpushed commits is left alone and
  reported, and `sync --force` is required to replace it. The check descends
  into that clone's own nested dependencies, so work in a grandchild counts too.

## Orphaned symlinks

When a dependency is removed from a child config, the symlink it had is left
behind. GitScale recognises its own links — relative, and pointing at a direct
child of the workspace root — and reports them as `orphan`:

- **Broken orphans** (the target is gone) are removed automatically by `sync`.
- **Orphans whose target still resolves** are only removed with `sync --force`,
  since something may still be using them.

Symlinks you created yourself, and anything absolute or pointing elsewhere, are
never touched.

## When resolution runs

| Command | Reads child configs | Creates symlinks | Checks out adopted revisions | Relinks / removes orphans |
|---|---|---|---|---|
| `clone` | yes | yes | yes | no |
| `pull` | yes | yes | no | no |
| `sync` | yes | yes | yes (via its clone step) | yes |
| `status` | yes | no | no | no |
| `clean` | yes | no | no | no |

---

[← 2.2 Status](status.md) · [Contents](README.md) · [Next → 2.4 Everyday workflow](workflow.md)

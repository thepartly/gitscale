# 2.8 Cleaning

- [What it does](#what-it-does)
- [The dry run](#the-dry-run)
- [Selecting repositories](#selecting-repositories)
- [What is always kept](#what-is-always-kept)
- [Exclusions](#exclusions)
  - [Per-repo `[clean]`](#per-repo-clean)
  - [`--exclude` on the command line](#--exclude-on-the-command-line)
- [What is skipped](#what-is-skipped)
- [Nested repositories GitScale does not manage](#nested-repositories-gitscale-does-not-manage)

## What it does

`gitscale clean` removes untracked files from the workspace repository and from
each sub-repository, the way `git clean -xd` does in one repo — ignored files
included. That is why the declared checkouts and GitScale's own symlinks are
[always kept](#what-is-always-kept) explicitly: it makes
[gitignoring `imports/`](dependencies.md#ignoring-the-checkout-directory) safe
here, even though a hand-run `git clean -xdf` at the root would delete the whole
workspace.

```
gitscale clean                  # dry run: list what would be removed
gitscale clean -f               # remove it
gitscale clean -f core          # just this repo
gitscale clean -f .             # just the workspace repo
gitscale clean -f -e 'dist/'    # keep dist/ in every repo cleaned
```

Each repository is cleaned with its own exclusion set: the workspace repository,
then every declared checkout, each of them exactly once — a repository reachable
both as a checkout and through a recursive dependency's symlink is not cleaned
twice.

## The dry run

Without `-f`, nothing is touched and everything that *would* go is listed:

```
Clean (dry run — nothing removed; pass -f to delete)
  . — nothing to remove
  core
    target/
    .env
  imports/shared — skip (symlink)

2 paths in 1 repo.
```

Paths are listed as `git clean` reports them: relative to the repository they
belong to, not to the workspace. Skipped repositories name their reason, so it
is clear whether a repository was clean or simply not cleaned.

## Selecting repositories

Names are the directory keys from `.gitscale.toml`. The workspace repository
itself is addressed as `.`, the same name `status` gives it.

| Invocation | Cleans |
|---|---|
| `gitscale clean` | the workspace repository and every declared entry |
| `gitscale clean core` | just `core` |
| `gitscale clean .` | just the workspace repository |
| `gitscale clean . core` | both |

## What is always kept

Beyond whatever you exclude, clean never removes:

- **The declared checkouts, at every level.** A sub-repository directory is an
  untracked directory as far as the repository holding it is concerned, so an
  unguarded `git clean -xdf` at the root would delete the whole workspace — and
  one declared inside another repository (`core` and `core/vendor`) would go the
  same way when that repository is cleaned. Clean excludes them wherever they
  sit. This matters most for [artefact](dependencies.md#artefact) entries, which
  have no `.git` directory for git to recognise them by.
- **Managed symlinks.** The links GitScale plants for
  [recursive dependencies](recursive-dependencies.md) are untracked files to
  git. Orphaned GitScale symlinks are *not* protected — those are removed, as
  `sync` would.
- **`.gitscale.toml`**, even when it has not been committed yet. Deleting the
  file that defines the workspace is not something a clean should be able to do.

Because the protected symlink set comes from resolving the recursive
dependencies, a config that cannot be resolved makes `clean` fail rather than
guess.

## Exclusions

Patterns use `.gitignore` syntax and are handed to `git clean -e` unchanged, so
each one means exactly what the same text on a `.gitignore` line would: matching
at any depth unless anchored with a leading `/`, and directories only when it
ends in `/`. They are anchored at the root of the repository they apply to — not
at the workspace root.

```toml
[clean]
exclude = [".vscode", ".idea", ".env", "envs/", "tmp"]
```

`tmp` matches at any depth, `/tmp` only at the top, `envs/` only directories.

### Per-repo `[clean]`

**A `[clean]` table speaks for its own repository and nothing below it.** The
root's exclusions apply to the workspace repository's own working tree; a
sub-repository declares its own in the `.gitscale.toml` in its checkout. A
repository that is not a workspace can carry a config with nothing but a
`[clean]` table in it:

```toml
# core/.gitscale.toml — core keeps its own scaffolding
[clean]
exclude = ["envs/", ".env"]
```

This is deliberate: a sub-repository knows what its own build leaves behind, and
a root config enumerating that on its behalf goes stale the moment the
sub-repository changes.

Reading a sub-repository's `[clean]` is gated on its `recursive` flag, like every
other nested-config read. A sub-repository's config that exists but does not
parse is an error rather than an empty keep-list — treating an unreadable file
as "keep nothing" would delete exactly the files it was written to protect.

### `--exclude` on the command line

```
gitscale clean -f -e 'dist/' -e '*.log'
```

Repeatable, and applied to **every** repository cleaned. Use it for a repository
you cannot add a config to; patterns that belong to one repository belong in that
repository's own `[clean]` table. An empty pattern, or one starting with `-`
(which git would read as a flag), is refused.

## What is skipped

| Reason reported | Why |
|---|---|
| `recursive = false` | Its config is not GitScale's to read, so its keep-list is unknown — and cleaning a repository without knowing what it wants kept is worse than leaving it alone |
| `artefact` | No working tree to clean; the directory's contents are managed by `pull` |
| `not cloned` | The directory does not exist |
| `symlink` | A deduped [recursive dependency](recursive-dependencies.md); the real checkout is cleaned under its own name |
| `not a git repository` | The directory exists but is not the top level of a repository. For the workspace root this is only reported when `.` was asked for by name — a workspace that is not itself a repository is an ordinary setup, not a problem |

## Nested repositories GitScale does not manage

A clone somebody made by hand inside a checkout is **reported, not deleted**:
clean does not pass `git clean`'s second `-f`. Remove those yourself. A stray
clone somebody forgot about is recoverable only while it still exists.

---

[← 2.7 CI authentication](ci-authentication.md) · [Contents](README.md) · [Next → 3. Configuration file reference](configuration.md)

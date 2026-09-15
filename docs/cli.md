# 4. Command line reference

- [Synopsis](#synopsis)
- [Global options](#global-options)
- [Output and exit status](#output-and-exit-status)
- [`gitscale clone`](#gitscale-clone)
- [`gitscale fetch`](#gitscale-fetch)
- [`gitscale pull`](#gitscale-pull)
- [`gitscale push`](#gitscale-push)
- [`gitscale sync`](#gitscale-sync)
- [`gitscale commit`](#gitscale-commit)
- [`gitscale clean`](#gitscale-clean)
- [`gitscale status`](#gitscale-status)
- [`gitscale add`](#gitscale-add)
- [`gitscale remove`](#gitscale-remove)
- [`gitscale cache`](#gitscale-cache)
- [`gitscale hook`](#gitscale-hook)

## Synopsis

```
gitscale [OPTIONS] <COMMAND> [ARGS...]
git scale [OPTIONS] <COMMAND> [ARGS...]
```

Installing GitScale provides two binaries: `gitscale`, and `git-scale`, which
lets git dispatch to it. `git scale status` and `gitscale status` are the same
command.

| Command | Purpose |
|---|---|
| [`clone`](#gitscale-clone) | Create missing checkouts, or bootstrap a workspace from a URL |
| [`fetch`](#gitscale-fetch) | Update remote state without touching working trees |
| [`pull`](#gitscale-pull) | Bring every checkout up to date, cloning what is missing |
| [`push`](#gitscale-push) | Push each writable checkout |
| [`sync`](#gitscale-sync) | clone + reconcile remotes + pull + relink + push |
| [`commit`](#gitscale-commit) | Commit across the workspace with one message |
| [`clean`](#gitscale-clean) | Remove untracked files, safely |
| [`status`](#gitscale-status) | Report the state of every declared repository |
| [`add`](#gitscale-add) / [`remove`](#gitscale-remove) | Edit `.gitscale.toml` |
| [`cache`](#gitscale-cache) | Inspect and maintain the object cache |
| [`hook`](#gitscale-hook) | Install or inspect GitScale's git hooks |

## Global options

Accepted by every command, before or after the subcommand.

| Option | Meaning |
|---|---|
| `-v, --verbose` | Verbose output: cache fallbacks, per-repository fetch lines, hook commands, cache totals |
| `--no-cache` | Talk to remotes directly, ignoring the [object cache](caching.md) |
| `-C, --root <PATH>` | Start the search for `.gitscale.toml` at `PATH` instead of the current directory. Accepted by every leaf command — for `cache` and `hook` that means the sub-subcommand: `gitscale cache status -C /path`, not `gitscale cache -C /path status` |
| `-h, --help` | Help for the command |
| `-V, --version` | Version (top level only) |

```
gitscale -v sync
gitscale sync -v
gitscale -C /path/to/project status
gitscale --version
```

## Output and exit status

The multi-repository commands print one line per repository:

```
  ok    imports/core
  skip  meta/art (artefact)
  FAIL  imports/utils: fatal: Could not read from remote repository.
```

`ok` and `skip` go to stdout, `FAIL` to stderr. On a terminal these are live
progress lines and repositories are processed in parallel; redirected, they are
printed sequentially as plain text.

Exit status is `0` on success and `1` on any failure, with a summary such as
`2 repo(s) failed to pull`. Skips are not failures.

## `gitscale clone`

```
gitscale clone [OPTIONS] [NAMES...]
gitscale clone [OPTIONS] <URL> [DIRECTORY]
```

Create the checkouts that do not exist yet, or — given a URL — bootstrap a whole
workspace. Full behaviour: [workflow → clone](workflow.md#clone).

| Argument | Meaning |
|---|---|
| `NAMES...` | Declared directories to clone. Empty means all |
| `URL [DIRECTORY]` | A repository to bootstrap from, and optionally the directory to create (default: the repository name) |

The first argument is read as a URL when it has a scheme, is an scp-style SSH
address, or is an absolute path.

```
gitscale clone
gitscale clone imports/core
gitscale clone https://github.com/org/root.git
gitscale clone git@github.com:org/root.git my-ws
```

## `gitscale fetch`

```
gitscale fetch [OPTIONS] [NAMES...]
```

Update remote state without modifying working trees: `git fetch` for git
entries (through the cache), a HEAD request for artefact entries. Checkouts that
do not exist are skipped. See [workflow → fetch](workflow.md#fetch).

## `gitscale pull`

```
gitscale pull [OPTIONS] [NAMES...]
```

Bring every selected checkout up to date, cloning anything missing first, then
re-create [recursive dependency](recursive-dependencies.md) symlinks and run the
[`post_sync` hook](hooks.md#post_sync). See
[workflow → pull](workflow.md#pull).

## `gitscale push`

```
gitscale push [OPTIONS] [NAMES...]
```

`git push` in each readwrite checkout. `readonly`, `artefact` and missing
checkouts are skipped.

## `gitscale sync`

```
gitscale sync [OPTIONS] [NAMES...]
```

| Option | Meaning |
|---|---|
| `--force` | Also relink unlinked clones that have local modifications, and remove orphaned symlinks whose target still resolves |

clone → reconcile remotes → pull → relink → push → `post_sync`. See
[workflow → sync](workflow.md#sync).

## `gitscale commit`

```
gitscale commit [OPTIONS] -m <MESSAGE> [NAMES...]
```

| Option | Meaning |
|---|---|
| `-m, --message <MESSAGE>` | **Required.** The commit message, used for every repository |

`git add -A` plus `git commit -m` in each selected checkout. Skips `artefact`
and `readonly` entries, missing checkouts, symlinked entries and repositories
that are already clean. With no names, the workspace repository is committed too
when it is the top level of a git repository. Nothing is pushed. See
[workflow → commit](workflow.md#commit).

## `gitscale clean`

```
gitscale clean [OPTIONS] [NAMES...]
```

| Option | Meaning |
|---|---|
| `-f, --force` | Actually delete. Without it, clean only lists what would go |
| `-e, --exclude <PATTERN>` | A path to keep, in `.gitignore` syntax, anchored at each repository's root. Applies to every repository cleaned; repeatable |

`.` addresses the workspace repository itself. See [cleaning](clean.md).

```
gitscale clean
gitscale clean -f
gitscale clean -f core
gitscale clean -f . -e 'dist/' -e '*.log'
```

## `gitscale status`

```
gitscale status [OPTIONS]
```

| Option | Meaning |
|---|---|
| `--fetch` | Fetch remote state before reporting |
| `-f, --format <FORMAT>` | `table` (default) or `json` |

Note that `-f` here is `--format`, not `--force`. Status takes no repository
names; it always reports everything. See [status](status.md).

## `gitscale add`

```
gitscale add [OPTIONS] <DIRECTORY> <REPO_URL> <REVISION>
```

| Argument / option | Meaning |
|---|---|
| `DIRECTORY` | Where the checkout goes, relative to the config |
| `REPO_URL` | The repository URL |
| `REVISION` | Branch, tag or commit SHA. Required positionally; pass `""` to leave it unset |
| `--mode <MODE>` | `readwrite` (default), `readonly` or `artefact` |

Edits `.gitscale.toml` only — run `gitscale clone` afterwards. Creates a config
if none exists. Fails if the directory is already declared. See
[the caveat about rewriting](configuration.md#how-add-and-remove-rewrite-the-file).

```
gitscale add imports/core https://github.com/org/core.git main
gitscale add meta/svc https://github.com/org/svc.git main --mode artefact
```

## `gitscale remove`

```
gitscale remove [OPTIONS] <DIRECTORY>
```

Removes the entry from `.gitscale.toml`. Fails if it is not declared. The
directory on disk is left alone — delete it yourself.

## `gitscale cache`

```
gitscale cache <status|update|repair|compact> [OPTIONS]
```

| Subcommand | Purpose |
|---|---|
| [`status`](caching.md#cache-status) | What the cache holds, one row per repository. `-v` also lists every ref a mirror holds |
| [`update`](caching.md#cache-update) | Refresh entries without touching any checkout. Takes optional `NAMES...` |
| [`repair`](caching.md#cache-repair) | Re-create entries this workspace borrows from but that are gone. Takes optional `NAMES...` |
| [`compact`](caching.md#cache-compact) | Repack entries and evict the ones nothing used lately |

| Option | Subcommand | Meaning |
|---|---|---|
| `--keep-recent <PERIOD>` | `compact` | How recently an entry must have been used to be kept. Default `1month`; accepts `12h`, `30d`, `2 weeks`, `1y`, and a bare number as days |

`status` and `compact` work from anywhere — the cache belongs to the user, not
to a workspace. A config is used when there is one, so `[cache] dir` is honoured.
All four print `The object cache is off.` and do nothing under `--no-cache` or
`[cache] enabled = false`.

```
gitscale cache status
gitscale cache update imports/core
gitscale cache repair
gitscale cache compact --keep-recent 2weeks
```

## `gitscale hook`

```
gitscale hook <install|uninstall|status|run> [OPTIONS]
```

| Subcommand | Purpose |
|---|---|
| `install` | Install the `post-checkout` and `post-merge` hooks |
| `uninstall` | Remove them, restoring any hook that was displaced |
| `status` | Where hooks are installed, what they allow, and what shadows them |
| `run <NAME>` | Run the pull for a git hook. Invoked by the installed hook, not by you |

| Option | Subcommand | Meaning |
|---|---|---|
| `--local` | `install`, `uninstall` | This repository only (default) |
| `--global` | `install`, `uninstall` | The current user (`~/.gitconfig`) |
| `--system` | `install`, `uninstall` | Every user on this machine (`/etc/gitconfig`) |
| `--force` | `install` | Replace an existing `core.hooksPath` or an already-displaced hook |
| `--allow <PATTERNS>` | `install` | Comma-separated glob patterns naming the repositories whose `.gitscale.toml` `[hooks]` commands this hook may run, matched against `host/owner/repo`. **Required for `--global` and `--system`**; use `'*'` to allow every repository |

The scope flags are mutually exclusive. See [hooks](hooks.md#git-hooks) and the
[hook allowlist](hooks.md#the-hook-allowlist).

```
gitscale hook install --local
gitscale hook install --global --allow 'github.com/acme/*'
gitscale hook install --system --allow 'github.com/acme/*,git.internal.example/*'
gitscale hook status
gitscale hook uninstall --local
```

---

[← 3. Configuration file reference](configuration.md) · [Contents](README.md) · [Next → 5. Related tools](related-tools.md)

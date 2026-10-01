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
- [`gitscale resolve`](#gitscale-resolve)
- [`gitscale add`](#gitscale-add)
- [`gitscale remove`](#gitscale-remove)
- [`gitscale artefact`](#gitscale-artefact)
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
| [`clone`](#gitscale-clone) | Create missing checkouts, or clone a whole workspace from a URL |
| [`fetch`](#gitscale-fetch) | Update remote state without touching working trees |
| [`pull`](#gitscale-pull) | Bring every checkout up to date, cloning what is missing |
| [`push`](#gitscale-push) | Push each writable checkout |
| [`sync`](#gitscale-sync) | clone + reconcile remotes + pull + relink + push |
| [`commit`](#gitscale-commit) | Commit across the workspace with one message |
| [`clean`](#gitscale-clean) | Remove untracked files, safely |
| [`status`](#gitscale-status) | Report the state of every checkout, and how each got its revision |
| [`resolve`](#gitscale-resolve) | Show where resolution moved the root's revisions, and record them |
| [`add`](#gitscale-add) / [`remove`](#gitscale-remove) | Edit `.gitscale.toml` |
| [`artefact`](#gitscale-artefact) | Publish this repository's build output as an artefact, and see what the registry holds |
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
  ok    meta/art (artefact 3f2a9c1)
  skip  libs/shared (symlink)
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

Create the checkouts that do not exist yet, or — given a URL — clone a whole
workspace. Every checkout lands at the revision
[resolution](recursive-dependencies.md#how-a-revision-is-chosen) settles on,
worked out before anything is cloned, implicit dependencies included. Full
behaviour: [workflow → clone](workflow.md#clone).

The URL form is not the usual way to set a workspace up: with a
[git hook](hooks.md#git-hooks) installed, a plain `git clone` does it. Use this
where no hook applies.

| Argument | Meaning |
|---|---|
| `NAMES...` | Directories to clone, declared or implicit. Empty means all |
| `URL [DIRECTORY]` | A repository to clone the workspace from, and optionally the directory to create (default: the repository name) |

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
entries (through the cache); for artefact entries, the commit the revision
names and whether it has an image, recorded without downloading anything — a
commit with no image fails. Git entries with no checkout are skipped. Also
refreshes what resolution reads, so the next offline `status` sees the remotes
as they are now. When resolution itself fails, the declared entries are fetched
anyway and the command fails afterwards with the reason — alongside the count
of entries that failed to fetch, when some did. See [workflow → fetch](workflow.md#fetch).

## `gitscale pull`

```
gitscale pull [OPTIONS] [NAMES...]
```

Bring every selected checkout to the revision
[resolution](recursive-dependencies.md#how-a-revision-is-chosen) settles on
against the remotes now, cloning anything missing first — into an empty
directory too, while one holding files but no repository fails — then
re-create [recursive dependency](recursive-dependencies.md) symlinks and run the
[`post_sync` hook](hooks.md#post_sync). A checkout is moved to another revision
only when that loses nothing: changes to tracked files, or a detached HEAD no
ref holds, fail the entry and leave it as it was. See
[workflow → pull](workflow.md#pull).

## `gitscale push`

```
gitscale push [OPTIONS] [NAMES...]
```

`git push` in each readwrite checkout. `readonly`, `artefact` and entries
with no checkout are skipped.

## `gitscale sync`

```
gitscale sync [OPTIONS] [NAMES...]
```

| Option | Meaning |
|---|---|
| `--force` | Also relink unlinked clones that have local modifications, remove orphaned symlinks whose target still resolves, and remove checkouts nothing needs any more that have local modifications |

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
and `readonly` entries, entries with no checkout, symlinked entries and repositories
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
| `--fetch` | Fetch remote state before reporting, and resolve against it |
| `-f, --format <FORMAT>` | `table` (default) or `json` |
| `--why [DIR...]` | Instead of the table, every request for each checkout named and which one won — see [--why](status.md#why-a-checkout-has-its-revision---why). With no directories, every checkout more than one repository asks for |

Note that `-f` here is `--format`, not `--force`. Status takes no repository
names; it always reports everything. Without `--fetch` it resolves from what
is on this machine, and never touches the network. See [status](status.md).

## `gitscale resolve`

```
gitscale resolve [OPTIONS]
```

| Option | Meaning |
|---|---|
| `--write` | Write each resolved revision into the root's entry for it |
| `-C, --root <PATH>` | Workspace to resolve |

Resolve against the remotes and list the root entries whose revision
resolution raised:

```
$ gitscale resolve
imports/d   v1.2.0 → v1.5.0   raised by imports/c
1 entry would change; run with --write to update .gitscale.toml
```

`--write` records them, editing only those revisions: comments, key order and
every other table stay as they were. Entries without a revision, and entries
marked `override`, are left alone; implicit dependencies are never added.

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

## `gitscale artefact`

```
gitscale artefact <publish|show|list> [OPTIONS]
```

| Subcommand | Purpose |
|---|---|
| [`publish`](artefacts.md#artefact-publish) | Pack the files this repository's [`[artefact]`](configuration.md#artefact) table selects — one layer per group — and push them to its registry as the image for one commit. Run in the pipeline, after the build |
| [`show`](artefacts.md#artefact-show) | For each artefact entry: the image, the commit its revision names now, whether that commit is published and with what layers, what is installed, and how the two compare. Takes optional `NAMES...` |
| [`list`](artefacts.md#artefact-list) | The commits each artefact entry has images for, labelled with the branches and tags that name them now, and which one is installed. Takes optional `NAMES...` |

| Option | Subcommand | Meaning |
|---|---|---|
| `-C, --root <PATH>` | all | Where to look for `.gitscale.toml`: the producing repository's for `publish`, the workspace's for `show` and `list` |
| `--commit <SHA>` | `publish` | The commit to publish for, as a full SHA. Default: the CI job's commit (`CI_COMMIT_SHA`, `GITHUB_SHA`), else `HEAD` |
| `--force` | `publish` | Replace an image already published for this commit with different files. Without it that is an error; publishing the same files again is always a no-op |
| `--dry-run` | `publish` | List every file of every layer and the layer digests, and send nothing. Needs no registry and no commit — the way to check what the patterns select |

`show` and `list` change nothing: they ask the remote and the registry, and
read what is installed. Naming an entry that is not an artefact entry is an
error.

```
gitscale artefact publish
gitscale artefact publish --dry-run
gitscale artefact show
gitscale artefact show meta/app
gitscale artefact list meta/app
```

## `gitscale cache`

```
gitscale cache <status|update|adopt|repair|compact> [OPTIONS]
```

| Subcommand | Purpose |
|---|---|
| [`status`](caching.md#cache-status) | What the cache holds, one row per repository: mirrors, snapshots and artefact images. `-v` also lists every ref a mirror holds |
| [`update`](caching.md#cache-update) | Refresh entries without touching any checkout; an artefact entry downloads the image its revision names. Takes optional `NAMES...` |
| [`adopt`](caching.md#cache-adopt) | Relink the workspace root to the cache now, whatever `adopt_root` says, and say why when it will not |
| [`repair`](caching.md#cache-repair) | Re-create entries this workspace borrows from but that are gone, and check every cached artefact blob against its digest. Takes optional `NAMES...` |
| [`compact`](caching.md#cache-compact) | Repack entries and evict the ones nothing used lately, artefact images included |

| Option | Subcommand | Meaning |
|---|---|---|
| `--shared` | `adopt` | Adopt from a linked worktree, relinking the object store it shares with its main worktree and every sibling |
| `--keep-recent <PERIOD>` | `compact` | How recently an entry must have been used to be kept. Default `12months`; accepts any [humantime](https://docs.rs/humantime) period, such as `12h`, `30d`, `2 weeks`, `1y` or `1d 12h`; a month is 30.44 days. A bare `m` is refused as ambiguous: write `min` or `months` |

`status` and `compact` work from anywhere — the cache belongs to the user, not
to a workspace. A config is used when there is one, so `[cache] dir` is honoured.
All five print `The object cache is off.` and do nothing under `--no-cache` or
`[cache] enabled = false`.

```
gitscale cache status
gitscale cache update imports/core
gitscale cache adopt
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

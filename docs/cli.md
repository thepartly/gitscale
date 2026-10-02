# 4. Command line reference

- [Synopsis](#synopsis)
- [Global options](#global-options)
- [Output and exit status](#output-and-exit-status)
- [`gitscale fetch`](#gitscale-fetch)
- [`gitscale pull`](#gitscale-pull)
- [`gitscale push`](#gitscale-push)
- [`gitscale sync`](#gitscale-sync)
- [`gitscale commit`](#gitscale-commit)
- [`gitscale clean`](#gitscale-clean)
- [`gitscale status`](#gitscale-status)
- [`gitscale develop`](#gitscale-develop)
- [`gitscale upgrade`](#gitscale-upgrade)
- [`gitscale check`](#gitscale-check)
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
| [`fetch`](#gitscale-fetch) | Update remote state without touching working trees |
| [`pull`](#gitscale-pull) | Put every checkout where resolution says, checking out what is missing |
| [`push`](#gitscale-push) | Push the topic branch of the root and every checkout on it |
| [`sync`](#gitscale-sync) | pull + relink + push |
| [`commit`](#gitscale-commit) | Commit across the workspace with one message |
| [`clean`](#gitscale-clean) | Remove untracked files, safely |
| [`status`](#gitscale-status) | Report the state of every checkout, and how each got its revision |
| [`develop`](#gitscale-develop) | Put checkouts on the topic, writable — or take them off |
| [`upgrade`](#gitscale-upgrade) | Promote a topic's released repositories, raise dependencies, or record what resolution selected |
| [`check`](#gitscale-check) | The merge gate: fail while anything comes from a topic branch |
| [`add`](#gitscale-add) / [`remove`](#gitscale-remove) | Edit `.gitscale.toml` |
| [`artefact`](#gitscale-artefact) | Publish this repository's build output as an artefact, and see what the registry holds |
| [`cache`](#gitscale-cache) | Inspect and maintain the CI cache |
| [`hook`](#gitscale-hook) | Install or inspect GitScale's git hooks |

## Global options

Accepted by every command, before or after the subcommand.

| Option | Meaning |
|---|---|
| `-v, --verbose` | Verbose output: per-repository fetch lines, hook commands, images pruned |
| `--no-cache` | In CI, talk to remotes directly instead of through the [CI cache](stores.md#the-ci-cache) |
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

## `gitscale fetch`

```
gitscale fetch [OPTIONS] [NAMES...]
```

Update remote state without modifying working trees: every git entry's
[store](stores.md) is fetched, checked out or not; for artefact entries, the
commit the revision names and whether it has an image, recorded without
downloading anything — a commit with no image fails. In CI, git entries are
skipped. Also refreshes what resolution reads, so the next offline `status` sees the remotes
as they are now. When resolution itself fails, the declared entries are fetched
anyway and the command fails afterwards with the reason — alongside the count
of entries that failed to fetch, when some did. See [workflow → fetch](workflow.md#fetch).

## `gitscale pull`

```
gitscale pull [OPTIONS] [NAMES...]
```

Put every selected checkout where
[resolution](recursive-dependencies.md#how-a-revision-is-chosen) says against
the remotes now — detached at its pin, or on the [topic](topics.md) branch —
checking out anything missing first, into an empty directory too, while one
holding files but no repository fails. Then re-create
[recursive dependency](recursive-dependencies.md) symlinks, prune unused
images once a day, and run the [`post_sync` hook](hooks.md#post_sync). A
checkout is moved only when that loses nothing: uncommitted changes to tracked
files, or a detached HEAD no ref holds, fail the entry and leave it as it was.
See [workflow → pull](workflow.md#pull).

## `gitscale push`

```
gitscale push [OPTIONS] [NAMES...]
```

`git push -u origin <topic>` in the root and in every checkout on the
[topic](topics.md). Checkouts off the topic, artefacts and entries with no
checkout are skipped; off a topic nothing is pushed. See
[workflow → push](workflow.md#push).

## `gitscale sync`

```
gitscale sync [OPTIONS] [NAMES...]
```

| Option | Meaning |
|---|---|
| `--force` | Also relink unlinked checkouts that have local modifications, remove orphaned symlinks whose target still resolves, and remove checkouts nothing needs any more that have local modifications |

pull → relink → push → `post_sync`. See
[workflow → sync](workflow.md#sync).

## `gitscale commit`

```
gitscale commit [OPTIONS] -m <MESSAGE> [NAMES...]
```

| Option | Meaning |
|---|---|
| `-m, --message <MESSAGE>` | **Required.** The commit message, used for every repository |

`git add -A` plus `git commit -m` in each selected checkout on the
[topic](topics.md). Skips checkouts off the topic — naming the `gitscale
develop` that brings one with changes in — artefacts, entries with no checkout,
symlinked entries and repositories that are already clean. With no names, the
root repository is committed too. Nothing is pushed. See
[workflow → commit](workflow.md#commit).

## `gitscale clean`

```
gitscale clean [OPTIONS] [NAMES...]
```

| Option | Meaning |
|---|---|
| `-f, --force` | Actually delete. Without it, clean only lists what would go |
| `-e, --exclude <PATTERN>` | A path to keep, in `.gitignore` syntax, anchored at each repository's root. Applies to every repository cleaned; repeatable |
| `--gc` | Compact instead: `git gc` in every store of the root, and drop the images nothing has used lately. Takes no names, `-f` or `-e`; refuses in CI |
| `--keep-recent <PERIOD>` | With `--gc`: how recently an image must have been used to be kept. Default `[clean] keep_recent`, else `3months` |

`.` addresses the workspace repository itself. See [cleaning](clean.md).

```
gitscale clean
gitscale clean -f
gitscale clean -f core
gitscale clean -f . -e 'dist/' -e '*.log'
gitscale clean --gc
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

## `gitscale develop`

```
gitscale develop [OPTIONS] <DIR>...
```

| Option | Meaning |
|---|---|
| `--stop` | Take the checkouts off the topic instead: back at their pins, their topic branches deleted |

Put each named checkout on the workspace's [topic](topics.md) — the root's
current branch — on a branch of that name from the commit it is at, writable.
A `replace` artefact becomes a checkout of its source at the same commit. A
directory is named by its checkout's path or by the path of a link a
repository has to it. Refuses off a topic, in CI, for an entry the root
overrides, and for a slot a dependency's pinned branch holds. `--stop` refuses
while the remote has the branch, or while it holds work no remote has. See
[`gitscale develop`](topics.md#gitscale-develop).

```
gitscale develop imports/core
gitscale develop imports/b/libs/d
gitscale develop --stop imports/core
```

## `gitscale upgrade`

```
gitscale upgrade [OPTIONS] [DIR...]
```

| Option | Meaning |
|---|---|
| `--resolved` | Write the revision resolution selected into the root's own entries, with no tag lookup |
| `--major` | Let a raise cross a semver major |
| `--commit` | Commit each edited `.gitscale.toml`, that file alone |
| `--dry-run` | Print the plan and change nothing |
| `-c, --create <BRANCH>` | The topic to create when none is active and a repository other than the root has to be edited |

With no directories: promote the topic's slots whose change a release now
holds, writing that release into the topic's configs and taking them off the
topic. With directories: raise each to its newest release in every config that
asks for it, developing the requesters on a topic — created with `git switch
-c` in the root when there is none. With `--resolved`: record what resolution
selected. Edits keep comments and key order. Refuses in CI. See
[promotion](topics.md#promotion-gitscale-upgrade),
[raising](topics.md#raising-a-dependency-gitscale-upgrade-dir) and
[`--resolved`](topics.md#writing-what-resolution-selected-upgrade---resolved).

```
gitscale upgrade --commit
gitscale upgrade imports/d
gitscale upgrade --resolved --dry-run
```

## `gitscale check`

```
gitscale check [OPTIONS]
```

The merge gate: fails while any checkout resolves from a topic branch rather
than a revision written in a config, naming each one and how to fix it. In a
merge request pipeline whose target is not a branch the root pins, it passes
without checking. Needs no history. See
[the merge gate](topics.md#topics-in-ci).

## `gitscale add`

```
gitscale add [OPTIONS] <DIRECTORY> <REPO_URL> <REVISION>
```

| Argument / option | Meaning |
|---|---|
| `DIRECTORY` | Where the checkout goes, relative to the config |
| `REPO_URL` | The repository URL |
| `REVISION` | Branch, tag or commit SHA. Required positionally; pass `""` to leave it unset |
| `--artefact <USE>` | `replace` or `overlay`: consume the repository's published [artefact](artefacts.md) |

Edits `.gitscale.toml` only — run `gitscale pull` afterwards. Creates a config
if none exists. Fails if the directory is already declared. See
[the caveat about rewriting](configuration.md#how-add-and-remove-rewrite-the-file).

```
gitscale add imports/core https://github.com/org/core.git main
gitscale add meta/svc https://github.com/org/svc.git main --artefact replace
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
gitscale cache <status|update|compact> [OPTIONS]
```

The per-user [CI cache](stores.md#the-ci-cache). A developer machine keeps
everything in the root's own [stores](stores.md) instead.

| Subcommand | Purpose |
|---|---|
| [`status`](stores.md#cache-status) | What the cache holds, one row per repository — snapshots and images — and every pin and image with its age |
| [`update`](stores.md#cache-update) | Add the pins and images a CI job of this workspace would take, touching no checkout. Takes optional `NAMES...` |
| [`compact`](stores.md#cache-compact) | Evict what nothing has used lately |

| Option | Subcommand | Meaning |
|---|---|---|
| `--keep-recent <PERIOD>` | `compact` | How recently an entry must have been used to be kept. Default `12months`; accepts any [humantime](https://docs.rs/humantime) period, such as `12h`, `30d`, `2 weeks`, `1y` or `1d 12h`; a month is 30.44 days. A bare `m` is refused as ambiguous: write `min` or `months` |

The commands work with `CI` set or not. `status` and `compact` work from
anywhere — the cache belongs to the user, not to a workspace; `compact` accepts
`-C` and ignores it.

```
gitscale cache status
gitscale cache update imports/core
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

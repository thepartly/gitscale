# 4. Command line reference

- [Synopsis](#synopsis)
- [Global options](#global-options)
- [Finding the workspace](#finding-the-workspace)
- [Directory arguments](#directory-arguments)
- [Git commands: `git scale <git command>`](#git-commands-git-scale-git-command)
- [Output](#output)
- [`git scale ls`](#git-scale-ls)
- [`git explain`](#git-explain)
- [`git topic`](#git-topic)
- [`git upgrade`](#git-upgrade)
- [`git scale sync`](#git-scale-sync)
- [`git scale clean`](#git-scale-clean)
- [`git scale gc`](#git-scale-gc)
- [`git scale require` / `unrequire`](#git-scale-require--unrequire)
- [`git scale prefer`](#git-scale-prefer)
- [`git scale check`](#git-scale-check)
- [`git scale hash`](#git-scale-hash)
- [`git scale artefact`](#git-scale-artefact)
- [`git scale cache`](#git-scale-cache)
- [`git scale hook`](#git-scale-hook)
- [`git scale skill`](#git-scale-skill)
- [Man pages](#man-pages)

## Synopsis

```
git scale [GLOBAL] <command> [ARGS]
git scale [GLOBAL] <git-command> [git args...] [--for DIR]... [--foreach] [--parallel[=N]] [--force-sync]
git topic [GLOBAL] ...          same as git scale topic
git upgrade [GLOBAL] ...        same as git scale upgrade
git explain [GLOBAL] ...        same as git scale explain
gitscale ...                    same as git scale ...
```

`cargo install gitscale` installs five binaries — `gitscale`, `git-scale`,
`git-topic`, `git-upgrade` and `git-explain` — so git dispatches `git scale`,
`git topic`, `git upgrade` and `git explain` to GitScale. `gitscale` takes the
same command line as `git scale`; CI scripts use it.

**Which commands are top-level.** `git topic`, `git upgrade` and `git explain`
are the everyday workflow commands: each names an idea of GitScale's own, and
clashes with no git command or common git add-on. Everything else stays under
`git scale`: git's own commands run across the workspace, the workspace's
views and maintenance (`ls`, `sync`, `clean`, `gc`), settings made once per
dependency (`require`, `unrequire`), and what pipelines mostly run (`check`,
`artefact`, `cache`, `hook`, `skill`). Each top-level command is one more
`git-<name>` binary on `PATH`, so a new command goes under `git scale` unless
it meets the same bar. Every top-level command also works as `git scale
<command>`.

| Command | Purpose |
|---|---|
| [`ls`](#git-scale-ls) (`list`) | Every checkout: its revision, how it was chosen, its state |
| [`explain`](#git-explain) | Every request behind a checkout's revision, and which one won |
| [`topic`](#git-topic) | Begin, go to, join, show and end topics |
| [`upgrade`](#git-upgrade) | Promote a topic's released repositories, or raise dependencies to their newest release |
| [`sync`](#git-scale-sync) | Put every checkout where resolution says, against the remotes now |
| [`clean`](#git-scale-clean) | Remove untracked files, keeping every checkout and link |
| [`gc`](#git-scale-gc) | Compact the stores and drop images nothing uses |
| [`require`](#git-scale-require--unrequire) / [`unrequire`](#git-scale-require--unrequire) | Add or remove a dependency, and place the workspace |
| [`prefer`](#git-scale-prefer) | How checkouts arrive: their sources, or the artefact of their release |
| [`check`](#git-scale-check) | The merge gate: fail while anything comes from a topic branch or a build |
| [`hash`](#git-scale-hash) | The source hash of the root or a checkout: what its own pipeline builds |
| [`artefact`](#git-scale-artefact) | Publish build output as an artefact, and see what the registry holds |
| [`cache`](#git-scale-cache) | Inspect and maintain the CI cache |
| [`hook`](#git-scale-hook) | Install or inspect GitScale's git hooks and man pages |
| [`skill`](#git-scale-skill) | Install, inspect or remove the agent skill |
| `help` | Help for a command |

Any other first word is a git command, run across the workspace — see
[git commands](#git-commands-git-scale-git-command).

## Global options

Accepted by every command: before or after the command name for GitScale's
own commands, before it for git commands, since everything after a git
command is git's.

| Option | Meaning |
|---|---|
| `-v, --verbose` | Verbose output: per-repository fetch lines, hook commands, images pruned |
| `--no-cache` | In CI, talk to remotes directly instead of through the [CI cache](stores.md#the-ci-cache) |
| `-C, --root <PATH>` | Run as if started in `PATH`: the search for the workspace starts there, and directory arguments are taken from there, as git's own `-C` |
| `--color <WHEN>` | `auto` (default), `always` or `never` — see [colour](#colour) |
| `-h, --help` | Help for the command |
| `-V, --version` | Version |

`git scale --help` and `git topic --help` are turned into a man page lookup by
git before GitScale runs — see [man pages](#man-pages). `git scale help
<command>` prints the same help as `-h`.

```
git scale -C /path/to/project ls
git scale sync -v
git scale --color never ls
```

## Finding the workspace

Every command, typed or run by a hook, starts by finding the workspace root:

1. Walk up from the current directory (or `-C PATH`) to the nearest directory
   holding `.gitscale.toml` at the top of a git repository.
2. If that repository is a **child** — a checkout GitScale made, whose git
   common dir is one of a root's stores, `<root common dir>/gitscale/repos/<name>.git`
   — keep walking up to the repository whose common dir is that root's. That
   is the workspace root. A child is never a workspace: a command typed inside
   one acts on the whole workspace.
3. A repository inside the workspace that is not a child — a clone made by
   hand — is a workspace of its own when it has a `.gitscale.toml`.
4. No root found: `not inside a GitScale workspace`, exit 1.

## Directory arguments

Every argument naming a checkout, `--for` included:

- is a path from the current directory, taken lexically, then made relative
  to the workspace root — `.` inside a checkout, `core` from `imports/`;
- names the checkout at that path, or the checkout a repository's dependency
  link at that path points to — `imports/b/libs/d` names `imports/d`;
- must stay inside the workspace: otherwise `DIR is outside the workspace`;
- `.` at the root names the root repository where a command takes it
  (`clean`, `sync`, `--for`); elsewhere `the root is not a checkout`;
- names no checkout: `DIR is not a checkout of this workspace`.

A command given no directory acts on the whole workspace.

## Git commands: `git scale <git command>`

```
git scale [GLOBAL] <git-command> [git args...] [--for DIR]... [--foreach] [--parallel[=N]] [--force-sync]
```

| Option | Meaning |
|---|---|
| `--for DIR` | Run only in these repositories. Repeatable. `.` is the root |
| `--foreach` | Add every checkout that is not on the topic |
| `--parallel[=N]` | Run in parallel, at most `N` at once; `N` defaults to the number of CPUs, at most 16 |
| `--force-sync` | The placement afterwards runs as `sync --force` |

These four are GitScale's wherever they appear — before the git command or
among its arguments — up to `--`; every other argument goes to git
unchanged. git's own commands use none of these names. `git scale pull --force` gives
git `--force`; `git scale pull --force-sync` forces the placement.

**Where it runs.** The root and every checkout on the topic — a checkout is on
the topic when it is on the branch resolution gives its slot. Off a topic, the
root alone. `--foreach` adds every checkout; `--for` names exactly the
repositories, and without `--foreach` naming a checkout that is not on the
topic fails before anything runs: `imports/d is not on the topic: git topic
join imports/d`.
Dependency links, and entries with no checkout of their own, are never among
them.

**In which order.** Dependencies before the repositories that ask for them,
following resolution's requests; the root last. Among repositories with no
order between them, by path. With `--parallel`, a repository starts once
everything it depends on has finished, and the root after every child.

**How.** In each repository, `git --no-pager -c push.autoSetupRemote=true
<git-command> [git args...]` — with `-c color.ui=always` when GitScale colours
its own output, so the two agree — and gitscale's git hook held off: what the
command moved is placed once, afterwards. Run in sequence, git gets the
terminal: an editor, `add -p` and a credential prompt all work. A repository
that fails does not stop the rest.

- **`commit`** skips a repository with nothing to commit, as `git commit
  --dry-run` answers; its header says `skip (nothing to commit)`. `--amend`
  and `--allow-empty` are never skipped.
- **`push`** sets the upstream of a branch on its first push.
- **Changes outside the run.** A checkout that is not on the topic and has
  uncommitted changes — the links GitScale planted aside — gets a line on
  stderr: `imports/d has changes but is not on the topic: git topic join imports/d`.

**Paging.** On a terminal, a command git pages — `log`, `show`, `diff`,
`grep`, `blame`, `shortlog`, `reflog`, or one `pager.<cmd>` turns on, after
alias lookup — writes all its output, headers included, to one pager: the one
`git var GIT_PAGER` names. `pager.<cmd> = false` turns it off; a pager of
`cat` or nothing is none. No other command is paged.

**In parallel.** Each repository runs with no terminal: stdin closed and
`GIT_TERMINAL_PROMPT=0`, so a credential prompt fails rather than hangs, and a
command that needs an editor fails with `(--parallel: no terminal; run without
it)`. On a terminal each repository has a live status line; its output is
printed whole, in order, once it finishes. `[forward] parallel = N` in the
root's `.gitscale.toml` makes `fetch`, `pull` and `push` parallel by default —
see [configuration](configuration.md#forward); `--parallel=1` runs in
sequence.

**Afterwards.** The command's name, after alias lookup — the first word of
`alias.<name>`, unless it is a shell alias (`!…`) — decides what follows:

| Command | Then |
|---|---|
| `pull` | [Placement](stores.md#placement), online: every checkout up to date, detached ones included |
| `fetch` | Every store of the root, and the image of every release taken as an artefact, refreshed. Nothing is placed |
| anything else | When it moved any repository's `HEAD`: placement, fetching only what resolution lacks. Otherwise nothing |

See [when resolution asks the remotes](recursive-dependencies.md#when-resolution-asks-the-remotes).
`--force-sync` with `fetch` is an error: `--force-sync has no effect with fetch`.

**Exit status.** `0` when every repository and the placement succeeded.
Otherwise `1`, with a last line on stderr naming each failure:

```
2 of 5 repositories failed: imports/core, .
placement failed: …
```

**Not run across the workspace.** `clean` is GitScale's own: a plain `git
clean -xdf` in the root deletes every checkout and artefact. `git scale
switch` and `git scale checkout` run like any other command, but checkouts may
fail or land on a stray branch until the placement puts them back: switch the
root with `git topic switch` or plain `git switch`.

```
git scale status
git scale add -A
git scale commit -m "PROJ-12 price cache"
git scale push
git scale log --oneline -5 --for imports/core
git scale log --oneline -1 --foreach
git scale --parallel pull
```

## Output

Applies to every command.

### Colour

Colour, and the icons below, are used on a stream when:

1. `--color always`: yes; `--color never`: no; otherwise
2. `NO_COLOR` is set, to anything, or `TERM=dumb`: no; otherwise
3. the stream is a terminal: yes.

stdout and stderr are decided apart, so piped output and CI logs get no escape
codes.

### Result lines

Multi-repository commands print one line per repository: `ok` and `skip` on
stdout, `FAIL` on stderr. With colour, each has an icon in front; the words
stay, for scripts and screen readers:

```
  ✔ ok    imports/core (PROJ-12-price-cache)
  ○ skip  imports/b/libs/d (symlink)
  ✘ FAIL  imports/d: fatal: Could not read from remote repository.
```

Without colour the lines are the words alone: `  ok    …`, `  skip  …`, `  FAIL  …`.
On a terminal they are live progress lines and repositories are processed in
parallel; redirected, they run in sequence. Exit status is `1` on any failure,
with a summary such as `2 repo(s) failed to place`. Skips are not failures.

### Headers

A git command run across the workspace prints a header before each
repository's output, and before the placement:

```
── 1/3 imports/core ──────────────────────────────────────────────
On branch PROJ-12-price-cache
Changes not staged for commit:
        modified:   src/cache.rs

── 2/3 imports/b ── skip (nothing to commit) ───────────────────────
── 3/3 . ─────────────────────────────────────────────────────────
On branch PROJ-12-price-cache

── placing ───────────────────────────────────────────────────────
  ✔ ok    imports/core (PROJ-12-price-cache)
```

`N/TOTAL` counts every repository the command runs in, skipped ones included,
in running order — padded to `TOTAL`'s width from 10 on. `.` is the root. On a
terminal the rule fills its width; otherwise it ends in 16 dashes. There is no
`placing` header when nothing is placed.

| Element | Colour |
|---|---|
| `✔ ok` | green |
| `○ skip` | dim |
| `✘ FAIL`, the failure summary | bold red |
| `hint:`, "has changes but is not on the topic" | yellow |
| Header rule, count and `skip (REASON)` | dim |
| Header name | bold |
| Spinner | cyan |

## `git scale ls`

```
git scale ls [GLOBAL] [--fetch] [-f, --format table|json]
```

`list` is the same command.

| Option | Meaning |
|---|---|
| `--fetch` | Resolve against the remotes first |
| `-f, --format <FORMAT>` | `table` (default) or `json` |

The table of every checkout — see [`git scale ls`](status.md). Takes no
directories. Resolves from what is on this machine unless `--fetch` is given —
in CI, what the job's placement fetched. While no agent skill is installed, the table prints a
[one-time hint](agents.md#the-hint) to install it.

## `git explain`

```
git explain [GLOBAL] [--fetch] [DIR...]
```

For each directory, every request for its checkout and which one won; with
none, every checkout more than one repository asks for. See
[`git explain`](status.md#why-a-checkout-has-its-revision-git-explain).

```
git explain imports/utils
git explain .            # inside a checkout
git explain
```

## `git topic`

```
git topic [GLOBAL]
git topic join [GLOBAL] <DIR>...
git topic join [GLOBAL] --dependants [DIR...]
git topic leave [GLOBAL] <DIR>...
git topic start [GLOBAL] [--from BRANCH] [--worktree | --no-worktree] [--dir DIR] <NAME>
git topic switch [GLOBAL] [--worktree | --no-worktree] [--dir DIR] <NAME>
git topic status [GLOBAL] [--fetch] [-f, --format table|json]
git topic list [GLOBAL] [--fetch] [-f, --format table|json]
git topic finish [GLOBAL] [--force] [NAME]
```

| Command | Does |
|---|---|
| `git topic` | Print the topic branch of the checkout the current directory is in; off a topic, nothing, exit 1 — and, at a terminal, why on stderr: `main is pinned, not a topic: git topic start NAME, or git topic switch NAME`. Offline |
| `join <DIR>...` | Put checkouts on the topic, from the commit each is at, writable — see [join and leave](topics.md#git-topic-join--leave) |
| `join --dependants [DIR...]` | Join the checkouts that ask for each `DIR` below its newest release; with none, those asking for the topic's changes, one level up |
| `leave <DIR>...` | Take them back to their pins, their topic branches deleted |
| `start <NAME>` | Begin a topic from the remote's default branch, or `--from BRANCH` — see [starting, switching and finishing](topics.md#starting-switching-and-finishing-topics) |
| `switch <NAME>` | Go to an existing branch of the root: a topic, a colleague's, or a pinned one |
| `status` | The current topic: what each joined repository still needs, what to merge next — see [where a topic stands](topics.md#where-a-topic-stands-git-topic-status) |
| `list` | Every topic of the root: worktree, joined checkouts, state |
| `finish [NAME]` | End a merged topic: back to the default branch, or its worktree removed; its local branches deleted, refused while one has commits no remote has — see [finishing](topics.md#starting-switching-and-finishing-topics) |

| Option | Command | Meaning |
|---|---|---|
| `--from <BRANCH>` | `start` | Start from this branch; from a topic, its joined checkouts come along |
| `--worktree`, `--no-worktree` | `start`, `switch` | In a worktree of its own, or in this one. Default: git config `gitscale.topic.worktree`, else a worktree when the root is a bare repository |
| `--dir <DIR>` | `start`, `switch` | Where a new worktree goes, from the current directory |
| `--fetch` | `status`, `list` | Fetch the joined repositories first |
| `-f, --format <FORMAT>` | `status`, `list` | `table` (default) or `json` |
| `--force` | `finish` | Abandon a topic: not merged, with checkouts still on it, which go back to their pins, or with commits no remote has, which are dropped. Uncommitted changes still refuse |

`join` and `leave` with no directory fail, as `git add` does: `no checkout
named`, with `hint: inside a checkout, use: git topic join .` when the current
directory is inside one. `start`, `switch` and `finish` refuse in CI, and work
from the directory holding a bare repository too.

```
git topic start PROJ-12-price-cache
git topic join imports/core
git topic join .                 # inside imports/core
git switch "$(git topic)"        # in a child: onto its topic branch with plain git
git topic status
git topic finish
```

## `git upgrade`

```
git upgrade [GLOBAL] [--major] [--commit] [--dry-run] [DIR...]
```

| Option | Meaning |
|---|---|
| `--major` | Let a raise cross a major |
| `--commit` | Commit each edited `.gitscale.toml`, that file alone |
| `--dry-run` | Print the plan and change nothing |

On a topic only. With no directories: promote the topic's slots whose change a
release now holds — writing that release into the topic's configs, deleting
their topic branches on their remotes, taking them off the topic. With
directories: raise each to its newest release — a tag holding its pin, on a
release branch where the repository names them — in the topic's configs that
ask for it; a requester off the topic is named with the `git topic join` that
brings it in. Edits keep comments and key order. With `--commit`, the last line
names the repositories that now have commits to push — a promotion edits other
topic checkouts' configs too:

```
to push: imports/b, . — git scale push
```

Refuses in CI. See [promotion](topics.md#promotion-git-upgrade) and
[raising](topics.md#raising-a-dependency-git-upgrade-dir).

```
git upgrade --commit
git upgrade imports/d
git upgrade .                    # inside imports/d
```

## `git scale sync`

```
git scale sync [GLOBAL] [--force] [DIR...]
```

| Option | Meaning |
|---|---|
| `--force` | Also relink checkouts with work, remove orphan links whose target still resolves, and remove checkouts nothing needs that hold work |

[Placement](stores.md#placement), online: resolve against the remotes, put
each checkout where resolution says, relink, prune unused images, and run
[`post_sync`](hooks.md#post_sync). Anything holding work is reported and kept
without `--force`, and the command exits 1. With directories, only those
checkouts are placed and relinked; `.` at the root is the whole workspace.
Nothing is pushed. The [hook](hooks.md#git-hooks) and CI run `sync`; typed, it
is rarely needed — `git scale pull` brings everything up to date.

## `git scale clean`

```
git scale clean [GLOBAL] [-n] [-f] [-d] [-x | -X] [-e PATTERN]... [-q] [DIR...]
```

| Flag | Meaning |
|---|---|
| `-n` | Dry run. Also what happens with neither `-n` nor `-f` |
| `-f` | Delete |
| `-d` | Untracked directories too |
| `-x` | Ignored files too |
| `-X` | Only ignored files |
| `-e PATTERN` | Keep, in `.gitignore` syntax, anchored at each repository's root. Repeatable |
| `-q` | Report only failures |

`git clean` with these flags in the root and each checkout (or the named ones;
`.` is the root), each once, never through a link. Always kept: every
checkout at every level, managed links, `.gitscale.toml`
and each repository's `[clean] exclude`. See [cleaning](clean.md).

```
git scale clean
git scale clean -fdx
git scale clean -fdx core
git scale clean -fd . -e 'dist/' -e '*.log'
```

## `git scale gc`

```
git scale gc [GLOBAL] [--keep-recent PERIOD]
```

| Option | Meaning |
|---|---|
| `--keep-recent <PERIOD>` | How recently an image must have been used to be kept. Default `[clean] keep_recent`, else `3months` |

`git gc` in every store of the root, and the images nothing has used within the
period dropped. Refuses in CI. See [compacting](clean.md#compacting-git-scale-gc).

## `git scale require` / `unrequire`

```
git scale require [GLOBAL] <DIR> <URL> [REVISION]
git scale unrequire [GLOBAL] <DIR>
```

| Argument / option | Meaning |
|---|---|
| `DIR` | Where the checkout goes, from the current directory |
| `URL` | The repository URL |
| `REVISION` | Branch, tag or commit SHA. Left out, no revision is written |

Edit the root's `.gitscale.toml` in place — comments, key order and tables
GitScale does not know about are kept — then place the workspace, online.
`require` creates the file when there is none, and fails when the directory is
declared; `unrequire` fails when it is not. `require` checks the new entry out;
`unrequire` leaves its checkout to the relink step, which removes it when it
holds nothing and reports it otherwise. See
[declaring dependencies](dependencies.md).

```
git scale require imports/utils https://github.com/acme/utils.git v2.1.0
git scale unrequire imports/utils
```

## `git scale prefer`

```
git scale prefer [GLOBAL] [DIR...]
git scale prefer [GLOBAL] --source|--artefact <DIR>...
```

| Option | Meaning |
|---|---|
| `--source` | Its sources: the default, so this removes the preference |
| `--artefact` | The published image of its release, in place of the sources |

Without a form: the preferences of the `DIR`s, or every preference with the
checkouts it applies to. With one — exactly one, and at least one `DIR` — the
preference for each checkout's repository, every major of it, in every
worktree of the root. It only records: the next placement applies it, and
`git scale ls` shows `source → artefact` meanwhile. See
[choosing how a checkout arrives](artefacts.md#choosing-how-a-checkout-arrives).

```
git scale prefer --artefact imports/billing-sdk
git scale prefer
git scale pull
```

## `git scale check`

```
git scale check [GLOBAL]
```

The merge gate: fails while any checkout resolves from a topic branch rather
than a revision written in a config, or any config pins a
[build](dependencies.md#build), naming each one and how to fix it. In a
merge request pipeline whose target is not a branch the root pins, it passes
without checking. Resolves online; needs no history. See
[the merge gate](topics.md#topics-in-ci).

## `git scale hash`

```
git scale hash [GLOBAL] [--committed] [-f, --format text|json] [DIR...]
```

The source hash of the root, or of each `DIR`: one SHA-256 of what that
repository's own pipeline builds at the commit it is at. Text: `<hash>  <DIR>`
per line, like `sha256sum`. Tag an image with it, and any workspace can tell
which image holds exactly the sources it has.

- **What it covers:** the repository's tree, and the tree of every repository
  its config reaches, each once — its config resolved as if it were the root,
  from this workspace's stores: on a topic branch with that topic's branches,
  at its pin with pins only. So a checkout hashes as its own pipeline does,
  whatever the workspace around it raised. An entry with `recursive = false`
  adds its own tree and nothing below it.
- **The input:** a line `<normalised url> <tree id>` per repository, the
  repository itself first and the rest sorted. Git's object ids, so a squash
  merge that changes no file keeps the hash.
- **Offline.** A source with uncommitted changes fails it:
  `imports/core has uncommitted changes; commit them, or --committed to hash its commit`.

| Option | Meaning |
|---|---|
| `--committed` | Hash each source's commit, its uncommitted changes left out |
| `-f, --format <FORMAT>` | `text` (default), or `json`: per `DIR`, the hash and each source's URL, its directory in this workspace, commit and tree |

`git scale ls --format json` carries the same hash as `source_hash` on each
checkout.

## `git scale artefact`

```
git scale artefact <publish|show|list> [GLOBAL] [OPTIONS]
```

| Subcommand | Purpose |
|---|---|
| [`publish [RELEASE]`](artefacts.md#artefact-publish) | Pack the files this repository's [`[artefact]`](configuration.md#artefact) table selects — one layer per group — and push them to its registry as the image of the sources checked out, tagged with their source hash, and with `RELEASE` when one is given. Run in the pipeline, after the build; tagging the commit in git is the pipeline's, after it |
| [`show [DIR...]`](artefacts.md#artefact-show) | For each checkout taken as an artefact, or each named: the release it is at, the image, whether that release is published, from which sources and with what layers, what is installed, and how the two compare |
| [`list [DIR...]`](artefacts.md#artefact-list) | The releases each such checkout has images of, newest first, with the source hash of each, and which one is installed |

| Option | Subcommand | Meaning |
|---|---|---|
| `RELEASE` | `publish` | The release this commit is: a version, such as `v1.4.0` or `v1-2026.10.06-153012`. The caller names it; one already tagging another commit is refused |
| `--force` | `publish` | Replace an image already published for these sources with different files, or move a release naming another image. Without it either is an error; publishing the same files again is always a no-op |
| `--dry-run` | `publish` | List every file of every layer and the layer digests, and send nothing. Needs no registry and no commit — the way to check what the patterns select |
| `--reuse` | `publish` | Pack nothing: release the image of these sources — the branch build a squash merge kept — as `RELEASE`, which it needs. Fails when there is no such image. See [releasing without rebuilding](artefacts.md#releasing-without-rebuilding) |

`-C` names the producing repository for `publish`, the workspace for `show` and
`list`. `show` and `list` change nothing: they ask the registry, and read what
is installed. A named checkout must be one taken as an artefact.

```
gitscale artefact publish
gitscale artefact publish --reuse v1-2026.10.06-153012
gitscale artefact publish --dry-run
git scale artefact show
git scale artefact list meta/app
```

## `git scale cache`

```
git scale cache <status|update|compact> [GLOBAL] [OPTIONS]
```

The per-user [CI cache](stores.md#the-ci-cache). A developer machine keeps
everything in the root's own [stores](stores.md) instead.

| Subcommand | Purpose |
|---|---|
| [`status`](stores.md#cache-status) | What the cache holds, one row per repository — snapshots and images — and every pin and image with its age |
| [`update [DIR...]`](stores.md#cache-update) | Add the pins and images a CI job of this workspace would take, touching no checkout |
| [`compact`](stores.md#cache-compact) | Evict what nothing has used lately |

| Option | Subcommand | Meaning |
|---|---|---|
| `--keep-recent <PERIOD>` | `compact` | How recently an entry must have been used to be kept. Default `12months`; accepts any [humantime](https://docs.rs/humantime) period, such as `12h`, `30d`, `2 weeks`, `1y` or `1d 12h`; a month is 30.44 days. A bare `m` is refused as ambiguous: write `min` or `months` |

The commands work with `CI` set or not. `status` and `compact` work from
anywhere — the cache belongs to the user, not to a workspace; `compact`
ignores `-C`.

```
gitscale cache status
gitscale cache update imports/core
gitscale cache compact --keep-recent 2weeks
```

## `git scale hook`

```
git scale hook <install|uninstall|status|run> [GLOBAL] [OPTIONS]
```

| Subcommand | Purpose |
|---|---|
| `install` | Install the `post-checkout` and `post-merge` hooks, and the [man pages](#man-pages) |
| `uninstall` | Remove the hooks, restoring any hook that was displaced |
| `status` | Where hooks are installed, what they allow, and what shadows them |
| `run <NAME>` | Place the workspace for a git hook. Invoked by the installed hook, not by you |

| Option | Subcommand | Meaning |
|---|---|---|
| `--local` | `install`, `uninstall` | This repository only (default) |
| `--global` | `install`, `uninstall` | The current user (`~/.gitconfig`) |
| `--system` | `install`, `uninstall` | Every user on this machine (`/etc/gitconfig`) |
| `--force` | `install` | Replace an existing `core.hooksPath` or an already-displaced hook |
| `--allow <PATTERNS>` | `install` | Comma-separated glob patterns naming the repositories whose `.gitscale.toml` `[hooks]` commands this hook may run, matched against `host/owner/repo`. **Required for `--global` and `--system`**; use `'*'` to allow every repository |
| `--child <PATH>` | `run` | The hook fired in this child: placement leaves it where git put it |

The scope flags are mutually exclusive. See [hooks](hooks.md#git-hooks), the
[hook allowlist](hooks.md#the-hook-allowlist) and
[the hook in a child](hooks.md#the-hook-in-a-child).

```
git scale hook install --global --allow 'github.com/acme/*'
git scale hook install --system --allow 'github.com/acme/*,git.internal.example/*'
git scale hook status
git scale hook uninstall --local
```

## `git scale skill`

```
git scale skill <install|status|remove> [GLOBAL] [OPTIONS]
```

| Subcommand | Purpose |
|---|---|
| `install` | Write the agent skill to `~/.agents/skills/gitscale/`, and to `~/.claude/skills/gitscale/` when `~/.claude` exists |
| `status` | Each location, and what is installed there |
| `remove` | Remove the copies GitScale wrote |

| Option | Subcommand | Meaning |
|---|---|---|
| `--force` | `install` | Replace a copy edited by hand, or a file GitScale did not write |
| `--force` | `remove` | Remove a copy edited by hand too |

Once installed, every interactive command outside CI keeps the skill current.
See [the agent skill](agents.md).

## Man pages

`git scale --help` is `man git-scale` to git, so GitScale writes its own man
pages: `gitscale.1`, `git-scale.1`, `git-topic.1`, `git-upgrade.1` and
`git-explain.1`, rendered from the command line definitions.

- **Where:** `<binary dir>/../share/man/man1/` — `~/.cargo/share/man/man1/`
  for a `cargo install`. `man` searches `<dir>/../share/man` for every `bin`
  directory on `PATH`, so no `MANPATH` setup is needed.
- **When:** `git scale hook install` writes them; any interactive run rewrites
  them once the version changed, as the agent skill is kept current.
- **A directory this user does not own, or cannot write**, is a system or
  package install, which ships its own pages: it is skipped, quietly.

---

[← 3. Configuration file reference](configuration.md) · [Contents](README.md) · [Next → 5. Related tools](related-tools.md)

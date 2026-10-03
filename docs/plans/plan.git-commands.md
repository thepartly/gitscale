# Plan: GitScale as git commands

Status: agreed, not started. Tag selection for `upgrade` is a separate plan:
[plan.improve-upgrade.md](plan.improve-upgrade.md).

- [Workflows](#workflows)
- [Decisions](#decisions)
- [Specification](#specification)
- [Implementation](#implementation)

## Workflows

Each step: what you want, as a comment; the GitScale command, with what it
does after it; then the native git that does the same, with `lacks:` for what
it misses and `note:` for anything else.

### One-time setup

```sh
# Install
cargo install gitscale    # binaries: gitscale, git-scale, git-topic, git-upgrade, git-explain

# Install the hook and the man pages
git scale hook install --global --allow 'github.com/acme/*'
#   or via native git:
#       not available
#       note: without the hook, run git scale sync after every clone, switch and pull
```

### Workflow 1: a plain clone

```sh
# Clone the workspace
git clone git@github.com:acme/app.git && cd app    # hook: stores in .git/gitscale/repos, children detached and read-only

# See the workspace
git scale ls    # every checkout: revision, how it was chosen, state
#   or via native git:
#       not available

# Start a topic
git topic start PROJ-12-price-cache    # hook: children with this branch (here or on the remote) join; the rest stay at their pins
#   or via native git:
#       git fetch && git switch -c PROJ-12-price-cache origin/main
#       note: same result

# Join core to the topic
git topic join imports/core    # core onto the topic from its pin, writable; inside it: git topic join .
#   or via native git:
#       git -C imports/core switch -c PROJ-12-price-cache
#       note: without -c when the branch exists here or on the remote; you pick which
#       lacks: turning an artefact into a source checkout; join's refusals (override, held slot)

# Edit app/ and imports/core/

# See what changed
git scale status    # every repo on the topic
git scale diff
#   or via native git:
#       git status
#       git -C imports/core diff
#       lacks: knowing which repos are on the topic

# Commit
git scale add -A
git scale commit -m "PROJ-12 price cache"    # repos with nothing to commit are skipped
#   or via native git:
#       git -C imports/core add -A && git -C imports/core commit -m "PROJ-12 price cache"
#       git add -A && git commit -m "PROJ-12 price cache"
#       note: one repo at a time

# Push
git scale push    # dependencies first, root last; upstream set on first push
#   or via native git:
#       git -C imports/core push -u origin HEAD
#       git push -u origin HEAD
#       note: children first, by hand; pushed the other way, the root's pipeline builds core at its pin

# Get up to date
git scale pull    # every checkout current, detached ones included
#   or via native git:
#       git -C imports/core pull && git pull
#       lacks: detached children; a moved branch revision, or a topic a colleague started in one, waits for git scale sync

# Work on a colleague's topic
git topic switch PROJ-9-colleague    # hook: children with PROJ-9 branches join
#   or via native git:
#       git fetch && git switch PROJ-9-colleague
#       note: same result

# See what is left to do on this topic
git topic status    # joined repos: ahead, pushed, promotion state; what to merge next
#   or via native git:
#       git -C <repo> status -sb, per joined repo
#       lacks: which repos are joined, promotion state, merge order

# Gate the root's merge, in the MR pipeline
git scale check    # fails while core comes from the topic
#   or via native git:
#       not available

# Promote core, once it is merged and tagged v2026.10.04
git upgrade --commit    # root's pin → v2026.10.04, core's topic branch deleted, core detached at the tag; ends with: to push: <repos>
#   or via native git:
#       edit the revision in every .gitscale.toml that asks for core, commit each
#       git -C imports/core switch --detach v2026.10.04
#       git -C imports/core branch -D PROJ-12-price-cache
#       lacks: finding the tag, checking it holds the change, finding every config that asks for core

# Push the pin bump
git push    # when only the root is left on the topic; otherwise git scale push
#   or via native git:
#       git -C <repo> push for each repo upgrade listed, then git push

# Finish, once the root is merged
git topic finish    # back on main, up to date, every child at its pin; topic branches deleted
#   or via native git:
#       git switch main && git pull
#       git -C imports/core branch -d PROJ-12-price-cache
#       lacks: the merged check; deleting each child's topic branch
```

### Workflow 2: a bare clone plus worktrees

Run from `app/`, the directory holding the bare repository.

```sh
# Clone the workspace
git clone --bare git@github.com:acme/app.git app/.git && cd app
#   or via native git:
#       the same, then once:
#       git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*' && git fetch
#       note: a bare clone fetches no new remote branches until told to; the first topic command does this itself

# Add the main worktree
git topic switch main    # worktree app/main; hook: stores in app/.git/gitscale/repos
#   or via native git:
#       git worktree add main main

# Start a topic in its own worktree
git topic start PROJ-13-retry    # worktree app/PROJ-13-retry from origin/main, same stores; prints: cd PROJ-13-retry
#   or via native git:
#       git fetch && git worktree add -b PROJ-13-retry PROJ-13-retry origin/main
#       note: with a [topic] prefix, you add it to the branch and drop it from the directory

# Join core to the topic
cd PROJ-13-retry
git topic join imports/core    # as in workflow 1
#   or via native git:
#       git -C imports/core switch -c PROJ-13-retry

# Edit, commit, push, promote: as in workflow 1

# Work on a colleague's topic
git topic switch PROJ-9-colleague    # its own worktree; hook: children with PROJ-9 branches join; prints: cd ../PROJ-9-colleague
#   or via native git:
#       git fetch && git worktree add ../PROJ-9-colleague PROJ-9-colleague

# See topics in progress
git topic list    # worktrees, joined children, not pushed / pushed / merged
#   or via native git:
#       git worktree list
#       lacks: joined children, state

# Finish, once merged
git topic finish    # checks it's merged, removes the worktree, deletes root and child topic branches; prints: cd ../main
#   or via native git, from ../main:
#       git worktree remove ../PROJ-13-retry && git branch -d PROJ-13-retry
#       lacks: the merged check; the children's topic branches stay in the stores
```

Layout:

```
app/
├── .git/                          bare repository, shared by every worktree
│   └── gitscale/repos/            one store per dependency, shared too
├── main/                          children detached at their pins
├── PROJ-13-retry/                 git topic start PROJ-13-retry
├── PROJ-9-colleague/              git topic switch PROJ-9-colleague
└── feature-blah/                  git topic start feature-blah   (branch andrey/feature-blah with [topic] prefix)
```

### Occasional

```sh
# Leave the topic
git topic leave imports/core    # back at the pin, topic branch deleted; inside it: git topic leave .
#   or via native git:
#       not available
#       note: needs the pin from resolution, and lacks the refusal when the branch holds unpushed work

# Raise a dependency to its newest release, everywhere it is asked for
git upgrade imports/d    # inside it: git upgrade .
#   or via native git:
#       edit every .gitscale.toml that asks for d
#       lacks: finding the release and every requester, putting each on a topic

# Promote the topic's released slots
git upgrade    # from anywhere
#   or via native git:
#       as git upgrade --commit in workflow 1

# See why a checkout has its revision
git explain imports/utils    # inside it: git explain .
#   or via native git:
#       not available

# See every checkout more than one repo asks for
git explain
#   or via native git:
#       not available

# Run a git command in some checkouts
git scale -s imports/core log --oneline -5
#   or via native git:
#       git -C imports/core log --oneline -5

# Run a git command in every checkout
git scale --all log --oneline -1    # detached ones included
#   or via native git:
#       git -C <dir> log --oneline -1, per checkout
#       lacks: the list of checkouts, implicit ones included

# Refresh remote state, moving nothing
git scale fetch    # every store and artefact info
#   or via native git:
#       git fetch, per checkout
#       lacks: stores with no checkout here, artefact info

# Pull in parallel
git scale --parallel pull    # output kept per repo
#   or via native git:
#       not available

# Clean
git scale clean -fdx    # checkouts, links and overlays always kept
#   or via native git:
#       DON'T: git clean -fdx in the root deletes every checkout and artefact

# Compact
git scale gc    # the stores, and images nothing uses
#   or via native git:
#       git gc in each store under .git/gitscale/repos/
#       lacks: dropping unused images

# Add or remove a dependency
git scale require imports/utils https://github.com/acme/utils.git v2.1.0    # edits .gitscale.toml in place, checks it out
git scale unrequire imports/utils    # edits .gitscale.toml in place, cleans up
#   or via native git:
#       edit .gitscale.toml, then git scale sync

# See the workspace with remote state refreshed
git scale ls --fetch
#   or via native git:
#       not available
```

### Forcing a cleanup

```sh
# Pull, then also relink checkouts with local work and remove orphans
git scale --force pull

# Not the same: git pull --force in each repo, normal cleanup
git scale pull --force
```

### CI on hosted runners (no hook)

```yaml
build:
  script:
    - gitscale sync
    - make
merge-gate:
  rules: [{ if: '$CI_PIPELINE_SOURCE == "merge_request_event"' }]
  script: [gitscale check]   # gates only merges into pinned branches, read from the MR target
```

## Decisions

1. **`git scale <git command>` runs git across the workspace.** Anything that
   is not a GitScale command is forwarded.
2. **Forwarding runs in the root and every checkout on the topic.** `-s`
   narrows that, `--all` adds detached checkouts. Dependencies run before the
   repos that need them, the root last.
3. **`push`, `fetch` and `commit` are dropped as GitScale commands**:
   forwarding does their job.
4. **Renamed:** `status` → `ls` (alias `list`), `status --why` → `explain`,
   `pull` → `sync` (no longer pushes), `add` / `remove` → `require` /
   `unrequire`, `clean --gc` → `gc`.
5. **Top-level git commands:** `git topic`, `git upgrade`, `git explain`.
   Each also works as `git scale …`.
6. **`clean` stays GitScale's**, with git's flags.
7. **Directory arguments are relative to the current directory**, in every
   command.
8. **A child is never a workspace.** A repository is a workspace only when it
   is cloned on its own, as a root. A hook firing in a child places the whole
   workspace, and commands typed in a child act on the workspace.
9. **`gitscale`** keeps the same command line as `git scale`.
10. **A clean break, no aliases** for old names: a hidden `status`, `push` or
    `commit` would stop those names from being forwarded.
11. **GitScale writes its own man pages**, so `--help` works after a
    `cargo install`.
12. **`git scale pull` brings everything up to date**, detached checkouts
    included. **`git scale fetch`** refreshes everything and moves nothing.
13. **`--parallel[=N]`** runs forwarded commands in parallel;
    **`[forward] parallel = N`** makes it the default for `fetch`, `pull` and
    `push`.
14. **One pager for the combined output**, only for commands git itself pages.
15. **Options before the git command are GitScale's, after it git's.**
16. **`recursive = false` stays** with its meaning; a hook in such a child
    behaves like any other child's.
17. **In CI, resolution always asks the remotes.** Branches never resolve
    from what an earlier job fetched.
18. **Colour and icons only on a terminal**, off with `NO_COLOR`, set with
    `--color`. Result lines keep their words; the `ls` table stops colouring
    piped output.
19. **`git topic join` / `leave` replace `develop` / `develop --stop`.**
    Plain `git switch` to the topic in a child works like `join`: the child's
    hook makes it writable. `join` and `leave` need a `DIR`; inside a slot
    that is `.`.
20. **`git topic start | switch | list | finish`** begin, go to, show and
    end topics: a branch in a plain clone, a worktree when the root is a bare
    repository with worktrees. They also work from the bare repository's
    parent, and set up a bare clone to fetch remote branches.
21. **`git topic status`** shows the current topic: its joined repos, what
    each still needs, and what to merge next. `git scale ls` stays the view
    of the whole workspace.

## Specification

### Terms

| Term | Meaning |
|---|---|
| Workspace root | The repository at the top of a workspace: a clone, or a worktree of one, holding `.gitscale.toml` at its top, that is not itself a child |
| Child | A checkout GitScale made: a worktree of a store, whose git common dir is `<root common dir>/gitscale/repos/<name>.git` |
| On the topic | A child whose current branch is the topic branch resolution gives its slot (the test `commit.rs` and `push.rs` use today). The root is on the topic when its branch is not pinned |
| Placement | What `sync` does: resolve, put every checkout where resolution says, check out what is missing, relink, prune images, run `post_sync`, and in CI `clean -f` each checkout moved |
| Online / offline | Online: resolution fetches every store it reads first. Offline: it reads the local stores only |
| Fetch on miss | Offline; then fetch each store that lacked something resolution asked for, and resolve again. Repeat while new gaps appear; each store is fetched at most once. What is still missing is an error |

### Finding the workspace

Every command, typed or run by a hook:

1. Walk up from the current directory (or `-C PATH`) to the nearest
   directory holding `.gitscale.toml` at the top of a git repository.
2. If that repository is a child (its common dir matches
   `*/gitscale/repos/*.git`), keep walking up until a `.gitscale.toml` whose
   repository's common dir is the child's `<root common dir>`. That is the
   workspace root.
3. A repository inside the workspace that is not a child (a clone made by
   hand) is a workspace of its own when it has a `.gitscale.toml`, as today.
4. No root found: `not inside a GitScale workspace`, exit 1.

`-C PATH` starts the search at `PATH`; it does not stop the walk past a
child.

### Directory arguments

Every argument naming a checkout (`DIR`), and `-s`:

- is a path relative to the current directory, resolved lexically, then made
  relative to the workspace root;
- must stay inside the workspace, else `DIR is outside the workspace`;
- names the checkout at that path, or the checkout a dependency's link at
  that path points to (as `develop` accepts today, and `topic join` keeps);
- `.` at the root names the root repository, where a command accepts it
  (`clean`, `sync`, `-s`); elsewhere `the root is not a checkout`;
- names no checkout: `DIR is not a checkout of this workspace`.

A command given no `DIR` keeps its workspace-wide meaning.

### Synopsis

```
git scale [GLOBAL] <command> [ARGS]
git scale [GLOBAL] [FORWARD] <git-command> [git args...]
git topic [GLOBAL] ...          same as git scale topic
git upgrade [GLOBAL] ...        same as git scale upgrade
git explain [GLOBAL] ...        same as git scale explain
gitscale ...                    same as git scale ...
```

**GLOBAL**, accepted by every command, before or after the command name for
GitScale commands, before it for forwarded ones:

| Option | Meaning |
|---|---|
| `-v, --verbose` | Verbose output |
| `--no-cache` | In CI, talk to remotes directly instead of through the CI cache |
| `-C, --root PATH` | Start the search for the workspace at `PATH` |
| `--color auto\|always\|never` | See [Colour](#colour). Default `auto` |
| `-h` | Help |
| `-V, --version` | Version |

`git <cmd> --help` is turned into a man page lookup by git before GitScale
runs; see [Man pages](#man-pages). `git scale help <command>` prints clap's
help.

**GitScale commands:** `ls`, `list`, `explain`, `topic`, `upgrade`, `sync`,
`clean`, `gc`, `require`, `unrequire`, `check`, `artefact`, `cache`, `hook`,
`skill`, `topic`, `help`. Any other first word is a git command and is forwarded.

### Forwarding

```
git scale [GLOBAL] [-s DIR]... [--all] [--parallel[=N]] [--force] <git-command> [git args...]
```

| Option | Meaning |
|---|---|
| `-s DIR` | Run only in these repositories. Repeatable. `.` is the root |
| `--all` | Add every checkout that is not on the topic |
| `--parallel[=N]` | Run in parallel, at most `N` at once; `N` defaults to the number of CPUs, capped at 16 |
| `--force` | The final placement runs as `sync --force` |

Everything from `<git-command>` on is passed to git unchanged.

#### Selection

1. Default: the root, and every child on the topic. Off a topic: the root
   alone.
2. `--all`: the root and every child.
3. `-s`: exactly the named repositories. Without `--all`, naming a child not
   on the topic fails before anything runs:
   `imports/d is not on the topic: git topic join imports/d`.
4. Always left out: dedup links (the checkout they point to is selected under
   its own path), artefact entries with no checkout, entries with no
   checkout.

#### Order

Dependencies before the repositories that ask for them, following
resolution's requests; the root last. Among repositories with no order
between them, by path. With `--parallel`, a repository starts once
everything it depends on has finished; the root starts after every child.

#### Running

For each selected repository, in order:

1. Record `HEAD` (commit and symbolic ref).
2. Print a [header](#headers).
3. Run, in that repository:

   ```
   git --no-pager -c push.autoSetupRemote=true [-c color.ui=always] <git-command> [git args...]
   ```

   with `GITSCALE_HOOK=1` in the environment, so no hook fires.
   `color.ui=always` only when GitScale colours its own output
   ([Colour](#colour)), so git and GitScale agree.
   Sequential runs inherit stdin, stdout and stderr.
4. A non-zero exit is a failure of that repository. The rest still run.

**`commit` skips repositories with nothing to commit**: before running,
`git commit --dry-run <git args>` (output discarded); exit 1 means nothing to
commit, and the repository's header carries `skip (nothing to commit)`
instead of running. `--amend` and `--allow-empty` are never skipped, since
`--dry-run` passes them.

**Changes outside the selection**: after the run, each child not selected
that has uncommitted changes (planted links excluded) gets one line on
stderr: `imports/d has changes but is not on the topic: git topic join imports/d`.

#### Paging

When stdout is a terminal, and the command (after alias lookup) is one git
pages — `log`, `show`, `diff`, `grep`, `blame`, `shortlog`, `reflog`, or one
the user's `pager.<cmd>` turns on — all output, headers included, is written
to one pager: the command `git var GIT_PAGER` prints. `pager.<cmd> = false`
turns it off. A pager of `cat` or empty means none. Other commands are never
piped: `commit`, `add -p` and `push` need the terminal.

#### Parallel

- Each repository runs with stdin closed and `GIT_TERMINAL_PROMPT=0`; a
  credential prompt fails instead of hanging. A command that needs an editor
  fails; the failure line adds `(--parallel: no terminal; run without it)`.
- On a terminal: one live status line per repository, as `sync` shows today
  (`run_entries` in `progress.rs`). Each repository's output is collected and
  printed whole, in [order](#order), once it finishes. Not on a terminal: the
  blocks only.
- Paging applies to the collected output.
- **Default:** `[forward] parallel = N` in the root's `.gitscale.toml` (an
  integer of at least 1) applies to `fetch`, `pull` and `push`, after alias
  lookup. Other commands run sequentially unless `--parallel` is given.
  `--parallel=N` overrides the config; `--parallel=1` runs sequentially.

#### At the end

The command's name, after alias lookup, decides what follows. Alias lookup:
`git config --get alias.<name>`; its first word, when it does not start with
`!`. A shell alias (`!…`) keeps its own name.

| Command | Then |
|---|---|
| `pull` | Placement, online |
| `fetch` | Refresh every store and every artefact entry's commit and image, as `gitscale fetch` does today. Nothing is placed |
| anything else | If any recorded `HEAD` changed: placement, fetch on miss. Otherwise nothing |

In CI every placement is online. `--force` makes the placement run as
`sync --force`; with `fetch` it is an error: `--force has no effect with
fetch`. Placement output follows the forwarded output, under its own
[header](#headers).

#### Exit status

0 when every repository and the placement succeeded. Otherwise 1, with a last
line on stderr: `2 of 5 repositories failed: imports/core, .` and, when
placement failed, `placement failed: …`.

#### Not forwarded

- `clean` is GitScale's own: a plain `git clean -xdf` in the root deletes
  checkouts and artefacts.
- `git scale switch` and `git scale checkout` run like any command, but
  children may fail or end up on a stray branch until the final placement
  puts them back. Switch the root with plain `git switch`.

### Output

Applies to every command.

#### Colour

Colour (and the icons below) is used for a stream when:

1. `--color always`: yes; `--color never`: no; otherwise
2. `NO_COLOR` is set (any value) or `TERM=dumb`: no; otherwise
3. the stream is a terminal: yes.

stdout and stderr are decided separately. **Fix:** the `ls` table today
colours unconditionally (`colorize` in `status.rs`), so piped output and CI
logs get escape codes; it follows this rule too.

#### Result lines

The `ok` / `skip` / `FAIL` lines of every multi-repository command
(`progress.rs`): with colour, an icon in front, words kept, since scripts and
tests `grep FAIL` and screen readers read words:

```
  ✔ ok    imports/core (PROJ-12-price-cache)
  ○ skip  imports/b/libs/d (symlink)
  ✘ FAIL  imports/d: fatal: Could not read from remote repository.
```

Without colour, exactly today's lines: `  ok    …`, `  skip  …`, `  FAIL  …`.

#### Headers

Before each repository's output in [forwarding](#forwarding), and before
placement:

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
  ○ skip  imports/b/libs/d (symlink)
```

- **Form:** `── N/TOTAL NAME ──…`. `NAME` is the repository's path, `.` for the
  root. A skipped repository's reason goes in its header: `── N/TOTAL NAME ──
  skip (REASON) ──…`, and nothing follows it.
- **Count:** `TOTAL` is every selected repository, skipped ones included, so
  `TOTAL/TOTAL` means the last. `N` follows print order: the
  [order](#order), also with `--parallel`. With 10 or more, `N` is padded to
  `TOTAL`'s width (` 2/12`).
- **On a terminal:** the rule fills the terminal width.
- **Not on a terminal:** the same text, ending in 16 `─`.
- **Placement** gets `── placing ──…`, followed by its result lines. No
  header when nothing is placed.

#### Colours

| Element | Colour |
|---|---|
| `✔ ok` | green |
| `○ skip` | dim: a skip is normal, not a warning |
| `✘ FAIL`, the failure summary | bold red |
| `hint:`, "has changes but is not on the topic" | yellow |
| Header rule and count | dim |
| Header name | bold |
| Header `skip (REASON)` | dim |
| Spinner | cyan, as today |
| `ls` table | as today, under the rule above |

### `ls` / `list`

```
git scale ls [GLOBAL] [--fetch] [-f, --format table|json]
```

Today's `status` table, unchanged apart from `--why` moving to `explain`.
Offline; `--fetch` resolves online first. Takes no directories.

### `explain`

```
git explain [GLOBAL] [--fetch] [DIR...]
```

Today's `status --why` output. With directories, every request for each and
which one won; with none, every checkout more than one repository asks for.
Offline; `--fetch` resolves online first.

### `topic`

```
git topic [GLOBAL]
git topic join [GLOBAL] <DIR>...
git topic leave [GLOBAL] <DIR>...
git topic start [GLOBAL] [--from BRANCH] [--worktree | --no-worktree] [--dir DIR] <NAME>
git topic switch [GLOBAL] [--worktree | --no-worktree] [--dir DIR] <NAME>
git topic status [GLOBAL] [--fetch] [-f, --format table|json]
git topic list [GLOBAL] [--fetch]
git topic finish [GLOBAL] [--force] [NAME]
```

**`git topic`** prints the topic branch of the checkout the current directory
is in:

- in the root: the root's branch, when it is a topic;
- in a child: its slot's topic branch, which differs from the root's for a
  slot with a checkout per major;
- off a topic (the root on a pinned branch, or detached): prints nothing,
  exits 1.

Offline, no network. Uses: `git switch "$(git topic)"` in a child, `git push
-u origin "$(git topic)"`, shell prompts, agents.

**`git topic join`** is today's `develop`: puts each checkout on its slot's
topic branch, from the commit it is at, writable; a `replace` artefact
becomes a checkout of its source. Same refusals as today: off a topic, in CI,
an entry the root overrides, a slot held at its pin.

**`git topic leave`** is today's `develop --stop`: back at the pin, the topic
branch deleted. Same refusals as today: the remote has the branch, or it
holds work no remote has.

`join` and `leave` with no `DIR` fail, as `git add` does: `no checkout
named`, with `hint: inside a checkout, use: git topic join .` when the
current directory is inside one.

Plain git reaches `join`'s state in every case, and the
[hook in the child](#the-hook-in-a-child) makes it writable:

| Topic branch in the child's store or on its remote | Plain git, in the child |
|---|---|
| exists nowhere | `git switch -c "$(git topic)"`: from the commit it is at, the pin |
| exists locally | `git switch "$(git topic)"` |
| exists on the remote only | `git switch "$(git topic)"`, tracking it |

`join` saves knowing which case applies, turns an artefact into a source
checkout, and makes its refusals.

#### Branch prefix

A team that names branches after their author turns it on in the root's
`.gitscale.toml`:

```toml
[topic]
prefix = "{user}/"
```

- `{user}` is git config `gitscale.user`, else `$USER` (`USERNAME` on
  Windows). Neither set: `start` fails with `set your name for branches: git
  config --global gitscale.user NAME`. `gitscale.user` is for a login that
  differs from the name used in branches, and for containers where `$USER`
  is `vscode`, `codespace` or `root`.
- `start NAME` creates branch `PREFIX` + `NAME`; a `NAME` already starting
  with the prefix is used as it is. `join`, `leave` and `finish` accept the
  name with or without it. `list` shows full branch names.
- No `[topic] prefix`: nothing is added or stripped.

#### Worktree layout

The root is in the **worktree layout** when its git common dir is a bare
repository (`core.bare = true`), as `git clone --bare` makes it. Then
`start`, `switch` and `finish` work on worktrees; otherwise on branches of
the one clone. `--worktree` / `--no-worktree` override for `start` and
`switch`; a personal default is git config `gitscale.topic.worktree`
(`true` / `false`), not `.gitscale.toml`, since it is one person's
preference.

**From the bare repository's parent** (`app/`, holding the bare `.git` and
no working tree), the `topic` commands still work: the layout is recognised
from `./.git` being bare, and `[topic] prefix` is read from `.gitscale.toml`
on the default branch (`git show HEAD:.gitscale.toml`). Other commands need a
worktree.

**Fetching remote branches.** `git clone --bare` writes no
`remote.origin.fetch`, so later fetches bring in no new remote branches and
no `origin/main`. The first `topic` command in the worktree layout sets
`remote.origin.fetch = +refs/heads/*:refs/remotes/origin/*` when it is
missing, fetches, and says so once: `configured origin to fetch remote
branches`.

A new worktree's directory name is the branch name without the
[prefix](#branch-prefix), with any remaining `/` turned into `-`, so all
topics sit at one level: `andrey/feature-blah` → `feature-blah`,
`andrey/feature/blah` → `feature-blah`; no prefix configured,
`andrey/feature-blah` → `andrey-feature-blah`. Two branches can map to one
name (`feat/retry`, `feat-retry`): when the directory exists, `start`
refuses with `DIR exists; choose another with --dir`. `--dir DIR` sets the
directory, relative to the current directory.

| Layout | Directory |
|---|---|
| Bare repository at `app/.git` | `app/NAME` |
| Plain clone at `projects/app`, with `--worktree` | `projects/app-NAME` |

```
app/                               bare layout, [topic] prefix = "{user}/"
├── .git/                          bare repository, shared by every worktree
│   ├── gitscale/repos/            one store per dependency, shared too
│   └── worktrees/                 git's records of each worktree
├── main/
├── feature-blah/                  git topic start feature-blah  → branch andrey/feature-blah
└── hotfix-retry/                  git topic start hotfix/retry  → branch andrey/hotfix/retry

projects/                          plain clone with --worktree
├── app/                           the clone; stores in app/.git/gitscale/repos
├── app-feature-blah/              git topic start --worktree feature-blah
└── app-hotfix-retry/
```

Not inside the clone (`app/.worktrees/…`): that needs a `.gitignore` entry,
and editors, build tools and test runners scanning the clone would pick the
worktrees up.

#### `git topic start`

1. Refuse in CI; refuse when the branch (with its [prefix](#branch-prefix))
   exists in the root locally or on its remote (`NAME exists: git topic
   switch NAME`); refuse a
   pinned branch name (`NAME is pinned, not a topic`); refuse an existing
   directory (`DIR exists; choose another with --dir`).
2. Base: `--from BRANCH`, else the root's default branch as its remote has
   it (`origin/main`), fetched first. Starting from a topic carries its
   joined children, as `git switch -c` from a topic does today.
3. Plain clone: `git switch -c NAME BASE` in the root; the hook places the
   children. Worktree layout: `git worktree add -b NAME DIR BASE`; the hook
   populates it from the shared stores. Then print
   `created DIR` and `cd DIR`, relative to the current directory.
4. Without the hook installed, `start` runs the placement itself.

#### `git topic switch`

Go to an existing branch of the root: local, on its remote (a colleague's
topic), or pinned (`main`). `NAME` is taken with or without the
[prefix](#branch-prefix).

1. Fetch the root. `NAME` exists nowhere: refuse with
   `NAME does not exist: git topic start NAME`. In CI: refuse.
2. Plain clone: `git switch NAME` in the root (tracking the remote branch
   when only the remote has it); the hook places the children.
3. Worktree layout: a worktree already on `NAME` → print `cd DIR`. Otherwise
   `git worktree add DIR NAME`, `DIR` named as for `start`, tracking the
   remote branch when only the remote has it; the hook populates it from the
   shared stores; print `created DIR` and `cd DIR`.
4. Without the hook installed, `switch` runs the placement itself.

Children follow placement: on a topic, every child whose store or remote has
the branch joins it.

#### `git topic status`

The current topic only: the root and every joined child.

```
topic PROJ-12-price-cache

  REPO           BRANCH                AHEAD  PUSHED  STATE
  .              PROJ-12-price-cache   2      no      waits on imports/core
  imports/core   PROJ-12-price-cache   3      yes     not tagged yet
  imports/b      PROJ-12-price-cache   1      yes     promoted → v2026.10.04

next to merge: imports/core
then: git upgrade --commit, git scale push
```

- `BRANCH`: the slot's topic branch (per-major slots differ).
- `AHEAD`: commits on the branch the pin doesn't have; for the root, commits
  the default branch doesn't have.
- `PUSHED`: `yes` when the remote has every commit, else `no`.
- `STATE`: for a child, the promotion state `upgrade` computes (`no change
  yet`, `not tagged yet`, `tagged TAG, no image yet`, `promoted → TAG`,
  `cannot tell`, `held`); for the root, `waits on REPOS` while any joined
  child is not promoted, else `ready to merge`.
- **Footer:** `next to merge` as `upgrade` prints it; `then:` the next
  command — `git upgrade --commit` once a child is promoted, `git scale push`
  when something is not pushed, `git topic finish` once the root is merged.
- Offline; `--fetch` fetches the joined repos' stores first. `--format json`
  for scripts and agents.
- Off a topic: `not on a topic`, exit 1.

#### `git topic list`

One row per topic of the root: every local branch of the root that is not
pinned, plus every root worktree's branch.

```
  TOPIC                 WORKTREE             JOINED                    STATE
▸ PROJ-12-price-cache   .                    imports/core              2 not pushed
  PROJ-13-retry         ../PROJ-13-retry     imports/core, imports/b   pushed
  feat/old              -                    -                         merged
```

- `▸` marks the current one.
- `WORKTREE`: the root worktree on that branch, relative to the current
  directory; `-` when none.
- `JOINED`: children with a local branch of that topic in their store
  (placement rule 1).
- `STATE`, for the root and every joined child together: `N not pushed` (commits
  no remote has), `pushed`, or `merged` (see `finish`). Offline;
  `--fetch` fetches the root and the joined children's stores first.
- `-f, --format table|json`, as `ls`.

#### `git topic finish`

Ends `NAME`, by default the current topic. Refuses in CI.

1. **Merged?** The root's topic branch is merged when merging it into the
   default branch (`origin/main`, fetched first) changes nothing — `git
   merge-tree` gives `origin/main`'s own tree, the content test `upgrade`
   uses, so squash and rebase merges count. Not merged: refuse with
   `NAME is not merged into main; --force to drop it`.
2. **Children still on the topic?** Each joined child must have left it
   (`git upgrade` does that once released). Otherwise refuse, naming them:
   `imports/core is still on the topic: git upgrade, or git topic leave
   imports/core`. `--force` does not skip this: a child's unreleased work
   would otherwise be lost silently.
3. **Uncommitted changes** in the root (or its worktree) refuse, `--force`
   or not.
4. Then:
   - plain clone: `git switch DEFAULT` and `git pull --ff-only`; the hook
     places every child at its pin;
   - worktree layout: `git worktree remove DIR`, and print `cd` to the main
     worktree when the current directory was inside `DIR`;
   - both: delete the root's local `NAME`, and each child store's local
     `NAME` that no remote branch or tag is missing.

`--force` finishes an unmerged topic: the root's local branch is deleted
with `git branch -D`. Its remote branch is never touched.

### `upgrade`

```
git upgrade [GLOBAL] [--resolved] [--major] [--commit] [--dry-run] [-c, --create BRANCH] [DIR...]
```

Unchanged apart from directory arguments, and: with `--commit`, the last line
names the repositories that now have commits to push —
`to push: imports/b, . — git scale push` — because a promotion also edits
other topic checkouts' configs. Which tag a pin moves to:
[plan.improve-upgrade.md](plan.improve-upgrade.md).

### `sync`

```
git scale sync [GLOBAL] [--force] [DIR...]
```

Placement, online. What `pull` does today, plus today's `sync` relinking, and
no push:

1. Resolve, online.
2. Place each selected checkout (the table in `workflow.md` → pull).
3. Relink: restore dedup links replaced by real checkouts, remove checkouts
   nothing needs, remove orphan links. Without `--force`, anything holding
   work is reported and kept, and the command exits 1.
4. Prune unused images, at most once a day.
5. Run `post_sync` when every step succeeded.
6. In CI, `clean -f` each checkout moved, as `pull` does today.

`--force` also relinks checkouts with work, removes orphan links whose target
still resolves, and removes checkouts nothing needs that hold work. The hook
and CI run `sync`; typed, it is rarely needed.

### `clean`

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

Runs `git clean` with these flags in the root and each checkout (or the named
ones; `.` is the root), each once, never through a link. Always kept: every
checkout at every level, an overlay's files, managed links,
`.gitscale.toml`, and each repository's `[clean] exclude`. Today's
`clean -f` is `clean -fdx`.

### `gc`

```
git scale gc [GLOBAL] [--keep-recent PERIOD]
```

Today's `clean --gc`: `git gc` in every store, and drop images not used in
`PERIOD` (default `[clean] keep_recent`, else `3months`). Refuses in CI.

### `require` / `unrequire`

```
git scale require [GLOBAL] [--artefact replace|overlay] <DIR> <URL> [REVISION]
git scale unrequire [GLOBAL] <DIR>
```

- Edit the root's `.gitscale.toml` in place with the editor `upgrade` uses:
  comments, key order and unknown tables are kept. `require` creates the file
  when there is none.
- `require` fails if `DIR` is declared; `unrequire` fails if it is not.
- `REVISION` is optional; omitted, no revision is written.
- Then placement of the whole workspace, online: `require` checks the new
  entry out; `unrequire` leaves its checkout to the relink step, which removes
  it when it holds nothing and reports it otherwise.

### `check`, `artefact`, `cache`, `skill`

Unchanged, apart from directory arguments.

### `hook`

Unchanged, except: `hook install` also writes the man pages, and the shim and
`hook run` follow [the hook in a child](#the-hook-in-a-child).

### The hook in a child

The shim today exits unless `.gitscale.toml` is at the repository's top. New
shim check, in order:

1. `GITSCALE_HOOK` set: exit (unchanged).
2. A file checkout (`post-checkout` with third argument `0`): exit
   (unchanged).
3. `git rev-parse --git-common-dir` matches `*/gitscale/repos/*.git`: a child.
   Run `gitscale hook run <hook> --child <child top> -C <child top>`.
4. Otherwise, `.gitscale.toml` at the top: run as today.
5. Otherwise exit.

`hook run` for a child:

- Finds the workspace root as in [Finding the workspace](#finding-the-workspace).
- Does nothing while the child is mid-operation: any of `rebase-merge`,
  `rebase-apply`, `MERGE_HEAD`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`,
  `BISECT_LOG` exists under its git dir (`git rev-parse --git-path`).
- Runs placement of the workspace, fetch on miss (online in CI), leaving the
  child's `HEAD` and files as they are. Links inside it are still planted.
- **The child on its slot's topic branch** (switched there with plain git):
  its files are made writable, as `topic join` does. Nothing is moved.
- **The child on any other branch:** left as it is, with a warning on stderr:
  `imports/core is on feat/x, not the topic PROJ-12-price-cache; the next
  placement moves it back to its pin`.
- Checks the allowlist against the workspace root's remote, since the root's
  `post_sync` is what runs.
- A child the root declares with `recursive = false` is handled the same way.

A hook in the root is unchanged: placement, online.

### Unlinked checkouts

A real checkout where a dedup link belongs. Since a child is never a
workspace, GitScale no longer makes them: running inside a child and naming an
entry no longer unlinks it. They still come from older layouts and hand-made
clones. `ls` reports `unlinked`; every placement relinks a clean one, and one
holding work is kept until `--force`.

### Network

| Placement | Network |
|---|---|
| Hook in the root (clone, `switch`, a merging `pull`, `worktree add`) | Online |
| `sync`, `check`, `require`, `unrequire`, forwarded `pull` | Online |
| Forwarded `fetch` | Online refresh, nothing placed |
| End of any other forwarded command; hook in a child | Fetch on miss |
| `ls`, `explain` | Offline; `--fetch` goes online |
| **Anything in CI** | **Online** |

**In CI** a failed fetch fails resolution. Today `load_refs` in `stores.rs`
falls back to the refs fetched before, with a warning; runners keep the build
directory between jobs, so that builds a stale commit. Outside CI the
fallback stays. The CI cache is unaffected: it is keyed by commit.

### Man pages

- **Pages:** `git-scale.1`, `git-topic.1`, `git-upgrade.1`,
  `git-explain.1`, `git-topic.1`, `gitscale.1`, rendered from the clap definitions with
  `clap_mangen` at runtime.
- **Where:** `<binary dir>/../share/man/man1/` — `~/.cargo/share/man/man1/`
  for a cargo install. `man` searches `<dir>/../share/man` for every `bin`
  directory on `PATH` (checked with `manpath` on Linux; check macOS), so no
  `MANPATH` setup is needed.
- **When:** `hook install` writes them; any interactive run rewrites them when
  the version changed, as the agent skill is kept current.
- **Not writable, or not owned by the user:** skip quietly. That is a system
  or package install, and the package ships the pages.

### Old commands

| Old | New |
|---|---|
| `gitscale status` | `git scale ls` |
| `gitscale status --why [DIR]` | `git explain [DIR]` |
| `gitscale pull [NAMES]` | `git scale pull`, or `git scale sync [DIR]` |
| `gitscale sync [--force]` | `git scale sync [--force]`; it no longer pushes |
| `gitscale push [NAMES]` | `git scale [-s DIR] push` |
| `gitscale fetch [NAMES]` | `git scale fetch` |
| `gitscale commit -m MSG [NAMES]` | `git scale [-s DIR] add -A && git scale [-s DIR] commit -m MSG` |
| `gitscale add DIR URL REV` | `git scale require DIR URL [REV]` |
| `gitscale remove DIR` | `git scale unrequire DIR` |
| `gitscale clean -f` | `git scale clean -fdx` |
| `gitscale clean --gc` | `git scale gc` |
| `gitscale develop DIR` | `git topic join DIR` |
| `gitscale develop --stop DIR` | `git topic leave DIR` |
| `gitscale upgrade` | `git upgrade` |
| CI: `gitscale pull` | `gitscale sync` |

## Implementation

### Code

| Where | Change |
|---|---|
| `Cargo.toml`, `src/bin/` | `git-topic`, `git-upgrade`, `git-explain`: insert the subcommand into argv, call `run_cli` |
| new `src/commands/topic.rs` | `topic` (print), from `topic::root` and the slot's resolved topic branch |
| `develop.rs` | Becomes `topic join` / `topic leave` |
| `src/commands/topic.rs` | Also `start`, `status`, `list`, `finish`; `status` reuses `promote::assess` and the merge order `upgrade` prints; `finish` reuses `promote::containment` for the merged test |
| `src/lib.rs` | New and renamed commands; `external_subcommand` with the forwarding options |
| `src/commands/forward.rs` (new) | [Forwarding](#forwarding) |
| `status.rs` | Becomes `ls`; `--why` moves to `explain.rs` |
| `pull.rs`, `sync.rs` | Merged into `sync`; fetch on miss; a child to leave in place; no push |
| `fetch.rs` | No longer a command; its refresh runs after a forwarded `fetch` |
| `push.rs`, `commit.rs` | Removed |
| `add.rs`, `remove.rs` | `require` / `unrequire`, editing in place, then placement |
| `clean.rs`, `git.rs` `clean_repo` | git's flags; `gc` split out |
| `hook.rs` | Shim and `hook run` for children; `install` writes man pages |
| `config.rs` `load_workspace` | [Finding the workspace](#finding-the-workspace); `[forward] parallel`; `[topic] prefix` |
| `stores.rs` `load_refs` | In CI, a failed fetch is an error |
| Commands taking directories | [Directory arguments](#directory-arguments) |
| new `src/man.rs` | Render and write man pages; called from `hook install` and the per-run refresh in `lib.rs` |
| new `src/output.rs` | [Colour](#colour) decision per stream, `--color`, header and result-line rendering |
| `progress.rs` | Result lines with icons and colours through `output.rs` |
| `status.rs` `colorize` | Only when [Colour](#colour) says so (bug fix) |
| `src/skill.md` | Rewritten for the new commands |

### Tests

- **forward:**
  - selection: default, `-s` (including `.` and an off-topic refusal), `--all`;
  - order: a dependency before its requester, the root last;
  - `commit` skips a repository with nothing to commit, but not `--amend`;
  - a first `push` sets the upstream;
  - the changes-outside-selection line;
  - `pull` places detached checkouts: a moved branch revision, a topic started
    remotely in a detached child;
  - `fetch` refreshes every store and moves nothing;
  - another command places only when a `HEAD` changed, fetching on miss;
  - an alias of `pull` counts as `pull`; a shell alias doesn't;
  - `--force` before the command relinks a checkout with work; after it,
    `--force` goes to git and the checkout is kept;
  - an editor gets the terminal; `log` goes through one pager; `commit` never
    does;
  - `--parallel`: output per repository not mixed, order kept, a credential
    prompt fails instead of hanging, an editor command fails naming the flag;
  - `[forward] parallel` makes `pull` parallel, leaves `log` sequential,
    `--parallel=1` overrides it;
  - exit status and summary when one repository fails;
  - headers: count includes skipped repositories, padded from 10, 16 dashes
    when not on a terminal, a `placing` header only when something is placed.
- **workspace finding:** from a child, from inside a child's subdirectory, a
  hand-made clone inside the workspace, outside any workspace.
- **hook in a child:** siblings placed after a pull that changes the child's
  config; a child without `.gitscale.toml`; the child stays where `git
  switch` put it; nothing mid-rebase and at a bisect step; fetch on miss;
  `recursive = false`; allowlist matched on the root's remote; a child
  switched to its topic branch becomes writable; one switched elsewhere is
  kept, with the warning.
- **topic:** root on a topic, root on a pinned branch (exit 1, no output), a
  child, a per-major slot's own branch; `join` and `leave` with no `DIR` fail,
  with the `.` hint only inside a checkout; today's `develop` tests move to
  `topic join` / `leave`.
- **topic switch:** plain clone to a colleague's remote branch, tracking it,
  children with that branch join; worktree layout adds a worktree, or prints
  `cd` when one exists; `main` works; a name that exists nowhere is refused
  with the `start` hint.
- **worktree layout:** commands run from the bare repository's parent; the
  fetch refspec is added once to a bare clone, after which a colleague's new
  branch is found.
- **topic start:** plain clone switches from `origin/main`; bare layout adds
  `app/NAME` and prints `cd`; `/` becomes `-`; `--worktree` on a plain clone
  gives `../app-NAME`; existing and pinned names refused; `--from` a topic
  carries its joined children; an existing directory refused, `--dir`
  chooses another.
- **branch prefix:** `{user}` from `gitscale.user`, else `$USER`; neither
  fails with the hint; a name already prefixed isn't prefixed twice; the
  directory drops the prefix; `join` / `finish` accept both forms; no
  `[topic] prefix` changes nothing.
- **topic status:** only the root and joined children; each promotion state;
  `waits on` / `ready to merge` for the root; the next command in the footer;
  off a topic exits 1; `--format json`.
- **topic list:** current marked; worktree paths; joined children; not
  pushed / pushed / merged, including a squash-merged topic.
- **topic finish:** squash-merged topic finishes; unmerged refused, `--force`
  drops it; a child still on the topic refused even with `--force`;
  uncommitted changes refused; plain clone ends on main with children at
  their pins; worktree removed and `cd` printed; remote branches untouched.
- **paths:** `.` in a slot; a relative path from a subdirectory; a link path;
  outside the workspace fails; `.` at the root where not accepted fails.
- **sync:** relinks a clean unlinked checkout; keeps one with work until
  `--force`; does not push.
- **clean:** each flag; no flag is a dry run; `-fdx` matches today's `-f`.
- **require / unrequire:** comments and unknown tables kept; the entry is
  checked out; an unrequired clean checkout is removed, one with work kept.
- **network:** in CI, a branch whose remote moved since the last job resolves
  to the new commit with old refs left in the build directory; an unreachable
  remote fails instead of using old refs; a forwarded command places online.
- **man:** written by `hook install`; rewritten on a new version; an
  unwritable directory skipped; `git scale --help` shows the page.
- **output:**
  - `ls` piped has no escape codes (the bug);
  - `NO_COLOR`, `TERM=dumb` and `--color never` turn colour and icons off;
    `--color always` turns them on when piped;
  - without colour, result lines are byte-for-byte today's;
  - forwarded git gets `color.ui=always` exactly when GitScale colours.
- **existing:** tests move with their commands; `push`, `fetch` and `commit`
  tests become forward tests; `links-016`, `links-017` and `fetch-005` change
  (from a child, the workspace is addressed, and naming no longer unlinks).
  Regenerate the test catalog.

### Docs

- `cli.md`: rewritten from [Specification](#specification).
- `workflow.md`, `topics.md`: the [Workflows](#workflows); `topics.md`'s
  *`gitscale develop`* section becomes *`git topic join` / `leave`*, and
  *Parallel topics* uses `git topic start` / `finish`, and *Promotion* shows
  `git topic status`; `workflow.md` drops
  the "Named, it is unlinked" row.
- `hooks.md`: the hook in a child.
- `recursive-dependencies.md`: *When resolution runs, and what it fetches*
  becomes the [Network](#network) table, which `workflow.md`, `hooks.md` and
  `cli.md` link to; *Unlinked checkouts* and *Deduplication by symlink* lose
  running inside a child as a workspace.
- `clean.md`: git's flags.
- `configuration.md`: `[forward]`, `[topic] prefix`, and the git config keys
  `gitscale.user` and `gitscale.topic.worktree`; drop the add/remove rewrite
  caveat.
- `status.md`: becomes `ls` and `explain`; its colour section points to the
  [Colour](#colour) rule.
- `cli.md`: `--color` under global options, and the [Output](#output) rules.
- `src/skill.md`: the new commands; `git scale pull` brings everything up to
  date, other forwarded commands don't refresh remotes.
- `README.md`, `agents.md`: examples.
- Release note: the [Old commands](#old-commands) table.

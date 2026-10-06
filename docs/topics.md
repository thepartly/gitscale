# 2.6 Topics: one change across several repositories

- [The idea](#the-idea)
- [A change across three layers](#a-change-across-three-layers)
- [Starting, switching and finishing topics](#starting-switching-and-finishing-topics)
  - [Branch prefix](#branch-prefix)
  - [Worktree layout](#worktree-layout)
- [Joining and leaving a topic](#joining-and-leaving-a-topic)
  - [Which branches are not topics](#which-branches-are-not-topics)
  - [Where each checkout goes](#where-each-checkout-goes)
  - [`git topic join` / `leave`](#git-topic-join--leave)
  - [A new branch from a topic](#a-new-branch-from-a-topic)
- [Inside a topic](#inside-a-topic)
- [Where a topic stands: `git topic status`](#where-a-topic-stands-git-topic-status)
- [Promotion: `git upgrade`](#promotion-git-upgrade)
- [Raising a dependency: `git upgrade <dir>`](#raising-a-dependency-git-upgrade-dir)
- [Topics in CI](#topics-in-ci)
- [Branch flows](#branch-flows)
- [Parallel topics](#parallel-topics)
- [Things to know](#things-to-know)

## The idea

A change that spans several repositories is a **topic**: one branch name, the
same in the root and in every repository the change touches. **The root's
current branch is the topic**, so nothing about a topic is stored apart from
the branches themselves. `git topic start`, `switch` and `finish` begin, go to
and end topics; plain `git switch` on the root works too.

Inside a topic, GitScale checks out the topic's branches instead of the
revisions the configs pin, and **no `.gitscale.toml` is edited** to test the
change. A CI pipeline on the branch does the same. Pins change only once a
layer is merged and released, and then `git upgrade` writes the new tag
into the configs that ask for it, bottom layer first, until the root merges.

The one rule behind every detail below: **no surprises between local and
CI.** A workspace on a topic and a pipeline on the same branch resolve alike.

## A change across three layers

The root pins B at `v1-2026.09.30`; B pins D at `v1-2026.09.28`. The change touches
D and B.

```
$ git topic start feat/price-cache
started feat/price-cache from origin/main
$ git topic join imports/d imports/b
imports/d on feat/price-cache, from v1-2026.09.28 (6be5fd3)
imports/b on feat/price-cache, from v1-2026.09.30 (1498d55)
…edit, build and test in the workspace…
$ git scale add -A
$ git scale commit -m "price cache"
$ git scale push
```

D is merged and its pipeline tags `v1-2026.10.01`:

```
$ git upgrade --commit
imports/b  no tag
imports/d  promoted → v1-2026.10.01
  imports/b/.gitscale.toml   libs/d  v1-2026.09.28 → v1-2026.10.01
  imports/d: deleted origin/feat/price-cache
  imports/d  left the topic: imports/d at v1-2026.10.01
  commit  imports/b/.gitscale.toml: pin imports/d v1-2026.10.01
next to merge: imports/b
to push: imports/b — git scale push
$ git scale push
```

B is merged and tagged; `git upgrade` bumps the root's pin of B; the root
merges, and `git topic finish` ends the topic.

## Starting, switching and finishing topics

| Command | Does |
|---|---|
| `git topic start <name>` | Begin a topic: a new branch of the root from the remote's default branch (`origin/main`, fetched first) — or a worktree of its own |
| `git topic start --from <branch> <name>` | From another branch; from a topic, the checkouts joined to it come along |
| `git topic switch <name>` | Go to an existing branch of the root: one of yours, a colleague's on the remote — tracking it — or a pinned one (`main`) |
| `git topic` | Print the topic of the checkout the current directory is in; nothing, exit 1, off a topic |
| `git topic list` | Every topic — each local branch the root does not pin, and the branch of each of its worktrees, `main` included — with its worktree, its joined checkouts, and `new` (nothing on it yet), `N not pushed`, `pushed` or `merged`; `-` for a pinned branch |
| `git topic finish [<name>]` | End a merged topic; `--force` abandons one |

After `start` and `switch`, the checkouts are placed: every one whose store or
remote has the branch joins it, the rest stay at their pins.

`start` refuses a name that exists here or on the remote (`NAME exists: git
topic switch NAME`), a name the root pins (`NAME is pinned, not a topic`), and
an existing directory for its worktree. `switch` refuses a name that exists
nowhere (`NAME does not exist: git topic start NAME`). Both refuse in CI.

`git topic` is for scripts, prompts and agents: in the root it prints the
root's branch; in a child, its slot's topic branch, which differs from the
root's for a slot with a checkout per major. It never uses the network.

```
git switch "$(git topic)"           # in a child: onto its topic branch
git push -u origin "$(git topic)"
```

**`finish`** ends a topic — the current one, or the one named:

1. **Merged?** The root's topic branch is merged when merging it into the
   default branch (`origin/main`, fetched first) changes nothing — the content
   test [promotion](#promotion-git-upgrade) uses, so squash and rebase merges
   count. Not merged: `NAME is not merged into main; --force to drop it`.
2. **Checkouts still on the topic** refuse it, naming each:
   `imports/core is still on the topic: git upgrade, or git topic leave
   imports/core`.
3. **Uncommitted changes** refuse it, `--force` or not — in the root, in each
   checkout going back to its pin, and in every checkout of a worktree being
   removed: `uncommitted changes in imports/core: commit or stash them (git
   scale stash -u), or discard them (git scale reset --hard && git scale clean
   -fd)`.
4. **Commits no remote has** refuse it: on a branch of the topic,
   commits no remote branch or tag holds, whose changes the default branch
   lacks — `feat@v7 in imports/core has 1 commit no remote has`, then `push
   them first, or --force to drop them`.

`--force` abandons a topic: it skips 1, 2 and 4, and the checkouts still on
the topic go back to their pins.

Then a plain clone goes back to the default branch and fast-forwards it, and
every checkout returns to its pin; a topic's own worktree is removed, with its
checkouts, and a `cd` to the main worktree when the command ran inside it.
The topic's local branches are deleted — the root's, and each store's `NAME`
and `NAME@vN` — and remote branches are never touched. A branch dropped with
commits no remote had is named with the commit it was at, as `git branch -D`
does:

```
switched to main
deleted PROJ-12 in ., imports/b
dropped PROJ-12 in imports/core (was 1a2b3c4): 2 commits no remote had
```

To come back to a topic later, do not finish it: `switch` to another, and its
branches stay. A finished topic comes back from its remote branches: `git
topic switch PROJ-12` tracks the root's, and each checkout follows its own.

### Branch prefix

A team that names branches after their author turns it on in the root's
`.gitscale.toml`:

```toml
[topic]
prefix = "{user}/"
```

- `{user}` is git config `gitscale.user`, else `$USER` (`USERNAME` on
  Windows). Neither set: `start` fails with `set your name for branches: git
  config --global gitscale.user NAME`. `gitscale.user` is for a login that
  differs from the name used in branches, and for containers where `$USER` is
  `vscode`, `codespace` or `root`.
- `start NAME` creates branch `PREFIX` + `NAME`; a name already starting with
  the prefix is used as it is. `switch` and `finish` take the name with or
  without it. `list` shows full branch names.
- No `[topic] prefix`: nothing is added or stripped.

### Worktree layout

The root is in the **worktree layout** when its git common dir is a bare
repository, as `git clone --bare` makes it. Then `start`, `switch` and
`finish` work on worktrees; otherwise on branches of the one clone.
`--worktree` / `--no-worktree` choose for one command; git config
`gitscale.topic.worktree` (`true` / `false`) is a personal default.

From the **bare repository's parent** — `app/`, holding the bare `.git` and no
working tree — `start`, `switch`, `list` and `finish` still work: the layout
is recognised from `./.git` being bare, and `[topic] prefix` is read from
`.gitscale.toml` on the default branch. Other commands need a worktree.

`git clone --bare` sets no `remote.origin.fetch`, so later fetches bring in no
new remote branches. The first topic command in the worktree layout sets it,
fetches, and says so once: `configured origin to fetch remote branches`.

A new worktree's directory is the branch name without the prefix, every
remaining `/` a `-`, so all topics sit at one level: `andrey/feature-blah` →
`feature-blah`, `andrey/feature/blah` → `feature-blah`; with no prefix
configured, `andrey/feature-blah` → `andrey-feature-blah`. When two branches
map to one name, `start` refuses the second: `DIR exists; choose another with
--dir`. `--dir DIR` sets the directory, from the current directory.

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

Not inside the clone (`app/.worktrees/…`): that needs a `.gitignore` entry, and
editors, build tools and test runners scanning the clone would pick the
worktrees up.

A topic's worktree deleted by hand leaves its branches behind: `git topic
list` shows the topic with `-` for its worktree, and `git topic switch NAME`
makes the worktree again, each joined checkout back on its branch.

## Joining and leaving a topic

| Command | Does |
|---|---|
| `git topic join <dir>...` | Put checkouts on the topic, writable, from the commit each is at |
| `git topic leave <dir>...` | Take checkouts off the topic, their topic branches deleted |
| `git topic switch <topic>` | Every checkout whose store or remote has the branch goes onto it |
| `git topic switch <pinned branch>` | Every checkout back at the revision its configs pin |

`git switch` on the root does the same as `git topic switch` once the
[git hook](hooks.md#git-hooks) places the checkouts; without the hook, run
`git scale sync` after it.

### Which branches are not topics

A branch the root **pins** is not a topic: on it, every checkout is at its pin.
By default the root pins only its remote's default branch — `main` and `master`
both, when there is no remote to ask. `[branches] pinned` names the pinned
branches instead:

```toml
[branches]
pinned = ["main", "staging", "release/*"]
```

A written list is exactly what is pinned: the default branch is not implied,
and `pinned = []` makes every branch a topic. A root on no branch at all has no
topic either.

### Where each checkout goes

On a topic, every [placement](stores.md#placement) places each checkout by
the first of:

1. **A local branch of the topic** in its store: on it, writable. This is a
   checkout someone joined — here, or in another worktree of the root.
2. **The remote's branch of the topic**: on a local branch tracking it,
   writable. This is how a colleague's `feat/price-cache` in E is used, and it
   is what CI does.
3. **Neither**: detached at its pin, read-only.

A checkout with uncommitted changes is never moved: the entry fails, the rest
carry on, and the command exits non-zero. Leaving a topic works the same way, so a
dirty checkout stays on the topic branch until its changes are committed or
discarded. Commits are never lost by a move: they stay on the topic branch in
the store, and `git switch` back brings them out again.

A slot the root holds with `override = true` never joins a topic, locally or
in CI. A slot whose repository has a checkout per major puts each major on its
own branch — see [two majors](#inside-a-topic).

### `git topic join` / `leave`

```
git topic join imports/d
git topic join imports/b/libs/d     # the same checkout, named by B's link to it
git topic join .                    # inside imports/d
git topic leave imports/d
```

`join` puts each named checkout on a branch of the topic's name, starting
from the commit it is at — its pin — and makes it writable. Directories are
paths from the current directory; see [directory arguments](cli.md#directory-arguments).

- An **artefact** checkout becomes a worktree of its source at the same commit,
  in place; see [artefacts → on a topic](artefacts.md#an-artefact-on-a-topic).
- An entry the root **overrides** is refused: `imports/d is held by override in
  .gitscale.toml; remove the override first`.
- A slot [held at its pin](#inside-a-topic) is refused.
- Off a topic, `join` says to start one: `git topic start <name>`.
- With no directory it fails, as `git add` does: `no checkout named`, with a
  hint to use `.` when the current directory is inside a checkout.

**`git topic join --dependants`** joins the checkouts whose configs ask for
others — never the root:

- **With directories:** the dependants of each that ask for less than its
  newest release. That is how a [raise](#raising-a-dependency-git-upgrade-dir)
  begins: the dependency need not be on the topic.
- **Without:** one level up from the topic's changes. A checkout on the topic
  *carries a change* when it has commits its pin does not, or uncommitted
  work. The dependants of each carrier with none of its dependants on the
  topic are joined. Once one of them is, that level is done, so a dependant
  taken off with `leave` stays off; a plain `join` brings it back.
- A dependant joined with nothing of its own carries no change until
  promotion commits its pin bump: run it again after each release to climb
  one more level.

```
$ git topic join imports/d          # changed, committed: a carrier
$ git topic join --dependants
imports/b on feat/price-cache, from v1-2026.09.30 (1498d55)
imports/c on feat/price-cache, from v1-2026.09.30 (77c0a1d)
$ git topic leave imports/c         # C stays at its release
$ git topic join --dependants
nothing to join
```

`leave` takes a checkout back to its pin and deletes its topic branch. It
refuses while the remote still has the branch — placement would follow it
again — and while the branch holds work no remote has.

**Plain git gets there too.** Switched to its topic branch with git, a child
is made writable by the [hook in the child](hooks.md#the-hook-in-a-child):

| The topic branch, in the child's store or on its remote | Plain git, in the child |
|---|---|
| exists nowhere | `git switch -c "$(git topic)"`: from the commit it is at, the pin |
| exists locally | `git switch "$(git topic)"` |
| exists on the remote only | `git switch "$(git topic)"`, tracking it |

`join` saves knowing which case applies, turns an artefact into a source
checkout, and makes its refusals.

### A new branch from a topic

`git switch -c feat/y` while on topic `feat/x` carries the topic along: every
checkout joined to `feat/x` gets a `feat/y` branch at the same commit, so a
change can be split or renamed without joining each repository again. `git
topic start --from feat/x feat/y` does the same from any branch.

```
$ git switch -c feat/price-cache-2
Switched to a new branch 'feat/price-cache-2'
  carry  imports/b → feat/price-cache-2
Pulling latest changes...
  ok    imports/b (on feat/price-cache-2)
  ok    imports/d
```

The carry happens in the placement the [git hook](hooks.md#git-hooks) runs after
the switch; without the hook, run `git scale sync` next. GitScale tells this
from git's own records — the new branch's reflog holds
nothing but its creation, and the root's last move was from the old branch to
it — so a branch created any other way starts empty, like any new topic.

## Inside a topic

**A topic slot is a local override.** Its revision is whatever its checkout
holds — work not pushed yet included — ahead of every request in the graph.
Its own `.gitscale.toml` is read from the working tree, so a dependency you add
on the branch, even uncommitted, resolves straight away. What resolution
would have chosen without the topic is kept as the slot's **pin**: what a
merge would ship.

**Pinned dependencies.** A repository whose own config pins the topic's branch
— `[branches] pinned` naming it — keeps what it asks for at its pins: a slot
only it asks for, and everything below that, stays put, and status says
`pinned by imports/b`. Any other requester's view is unchanged.

**Two majors.** When a repository has a checkout per major, the highest major
is on the topic's own branch and every other on `<topic>@v<major>` —
`feat/x@v1`, `feat/x@v0.4` — since one branch cannot say which major it means.
CI matches the same names.

**Git commands act on the topic.** `git scale <git command>` runs in the root
and every checkout on the topic, dependencies first — see
[git commands](cli.md#git-commands-git-scale-git-command). `git scale commit`
skips a repository with nothing to commit; `git scale push` sets each branch's
upstream on its first push. A checkout with changes that is not on the topic
gets a line saying how to bring it in:

```
imports/e has changes but is not on the topic: git topic join imports/e
```

**`ls` follows the topic.** The table starts with the topic and what may merge
next, and each topic row says where its change stands and what it waits on —
see [ls → topics](status.md#topics):

```
topic feat/price-cache · next to merge: imports/d
    REPO        PATH   AS         REF                EXPECTED           STATUS   RESOLUTION
✔   imports/b   -      source     feat/price-cache   feat/price-cache   ok       topic, waits on imports/d, no tag
✔   imports/d   -      source     feat/price-cache   feat/price-cache   ok       topic, no tag, implicit via imports/b
```

A topic branch cut from an older release than another repository now asks for
gets `behind v1-2026.09.30 wanted by imports/c: rebase it`. That needs history,
so it is worked out from the store, and never in CI.

## Where a topic stands: `git topic status`

The current topic only: the root and every joined checkout.

```
topic PROJ-12-price-cache

  REPO           BRANCH                AHEAD  PUSHED  STATE
  .              PROJ-12-price-cache   2      no      waits on imports/core
  imports/core   PROJ-12-price-cache   3      yes     no tag
  imports/b      PROJ-12-price-cache   1      yes     promoted → v1-2026.10.04

next to merge: imports/core
then: git upgrade --commit, git scale push
```

- `BRANCH`: the slot's topic branch — per-major slots differ.
- `AHEAD`: commits on the branch its pin does not have; for the root, commits
  the default branch does not have.
- `PUSHED`: `yes` when the remote has every commit; `-` when there is nothing
  to push.
- `STATE`: for a checkout, its [promotion state](#promotion-git-upgrade); for
  the root, `waits on REPOS` while a joined checkout with a change is not
  promoted, else `ready to merge`, and `merged into main` once it is.
- **Footer:** `next to merge`, as `upgrade` prints it; `then:` the next
  commands — `git upgrade --commit` once a checkout is promoted, `git scale
  push` when something is not pushed, `git topic finish` once the root is
  merged and no checkout is still on the topic.

Offline; `--fetch` fetches the joined repositories first. `--format json` is
for scripts and agents. Off a topic: `not on a topic`, exit 1.

## Promotion: `git upgrade`

`git upgrade` finds which topic slots have reached a release, writes those
releases into the topic's configs that ask for less, and takes the promoted
slots off the topic.

**No merge strategy is assumed.** A squash or a rebase rewrites commits, so
history cannot tell whether a branch was merged; content can. A slot is
promoted to **the newest release that contains the change**: `git merge-tree`
writes what merging the branch into the tag would give, and it is the tag's
own tree. Releases are asked newest first, so a newer hotfix cut beside it,
without the change, is passed over. This runs in each slot's store, which has
the history; never in CI.

**Which tags are releases.** A tag of the pin's kind and major, reachable from
a branch the repository [pins](configuration.md#branches), whose history holds
the pin's commit: a tag on a line split off before the pin would lose what the
pin had. By default a repository pins its default branch alone; one that cuts
releases elsewhere says so in its own config, for every workspace that depends
on it:

```toml
# core's .gitscale.toml
[branches]
pinned = ["main", "release/*"]
```

It is read from the repository's default branch, so a pin older than the list
follows it too. A list that names no branch of the repository is an error.

| State | Meaning | `upgrade` does |
|---|---|---|
| `no change yet` | The branch adds nothing to the pin | Nothing |
| `no tag` | No release holds the change | Nothing |
| `no release contains <pin>` | No release holds the pin itself: its tag was moved, or it was cut on no release branch | Nothing; raise it explicitly with `git upgrade <dir>` |
| `tagged <tag>, no image` | Released, but the repository publishes artefacts and the release has no image | Nothing |
| `promoted → <tag>` | The tag holds the change | Edits the configs, then the slot leaves the topic |
| `cannot tell` | Merging conflicts: a later commit in the tag rewrote the same lines | Nothing; bump it explicitly with `git upgrade <dir>` |
| `held (… uncommitted)` | Work no tag can hold | Nothing; commit or discard it first |

Commits on the branch that nobody pushed do not hold a slot back: once the tag
holds their content, they are what a squash left behind.

`git upgrade` works on a topic only: off one, both forms fail with `not on a
topic: git topic start NAME`.

**What it edits.** For each promoted slot, every config of the topic — the
root's, and each topic checkout's working tree — that asks for it below the
new tag gets the tag written in, in the tag's own spelling. Requesters outside
the topic are left alone: they keep their releases, and highest-wins
resolution lifts them in this workspace. An override in a requester is
reported, not changed. The file is edited in place, so comments and key order
survive.

**The remote branch.** A promoted slot's topic branch left on its remote keeps
matching by name, so the pipeline of the pin bump would still build from it.
Before anything changes, `upgrade` asks each promoted slot's remote for the
branch; the release must hold everything at its tip, or the whole `upgrade`
fails with nothing changed:

```
Error: imports/d: origin/feat/price-cache has changes v1-2026.10.01 does not hold; merge or drop them, then run again
```

Then each branch is deleted — only while it is still at the tip checked, so a
push made meanwhile fails the deletion instead of being lost — and only then
are the configs edited. A refused deletion, by permissions or a protected
branch, fails `upgrade` with no config edited; branches already deleted stay
deleted, since their releases hold them, and running it again continues.
`--dry-run` lists what it would delete.

**Leaving.** The slot's local topic branch is deleted and it is checked out
detached at the tag, read-only; an artefact gets the tag's image back.

**The merge order.** A topic repository is ready to merge when no topic slot it
asks for, directly or further down, is still unpromoted; the root merges last.
`upgrade`, `git topic status` and `git scale ls` print the next ones.

| Option | Meaning |
|---|---|
| `--commit` | Commit each edited `.gitscale.toml` — that file alone, whatever else the repository has changed — as `pin <dir> <tag>` |
| `--dry-run` | Print the plan; change nothing |

Pushing is left to `git scale push`; with `--commit`, the last line names what
to push: `to push: imports/b, . — git scale push`.

## Raising a dependency: `git upgrade <dir>`

`git upgrade imports/d` — or `git upgrade .` inside it — raises D in the
topic's configs:

1. The new revision is the **newest release of D's kind and current major**,
   semver or calendar, among the [releases](#promotion-git-upgrade) that hold
   the pin. `--major` crosses majors, and drops that last condition, since a
   new major is often cut on a line of its own — on a pinned branch still.
   A pre-release is picked only when the current pin is already a pre-release
   — the rule npm and Cargo follow. Without access to D's sources, the newest
   version its registry has: releases are cut in order on D's release
   branches, so the newest holds every one before it.
2. **The configs on the topic are edited**: the root's, and those of the
   requesters joined to it. A requester off the topic is named, with the
   command that joins it, and left as it is.
3. A requester asking for no version (a branch) is reported, not changed. A
   requester the root overrides is reported. If D itself is overridden,
   nothing is edited.

`git topic join --dependants imports/d` joins every requester that asks for
less, so a raise everywhere is:

```
$ git topic start upgrade-d
$ git topic join --dependants imports/d
imports/b on upgrade-d, from v1-2026.09.30 (1498d55)
imports/c on upgrade-d, from v1-2026.09.30 (77c0a1d)
$ git upgrade imports/d
imports/d   v1-2026.09.28 → v1-2026.10.01
  imports/b/.gitscale.toml   libs/d  v1-2026.09.28 → v1-2026.10.01
  imports/c/.gitscale.toml   libs/d  v1-2026.09.30 → v1-2026.10.01
next to merge: imports/b, imports/c
```

The cascade then runs as for any change: B and C merge and are tagged, and
`git upgrade` bumps the root's pins of them.

`upgrade` in all its forms works on a developer machine; in CI it refuses.

## Topics in CI

A pipeline on a topic branch resolves the topic exactly as a workspace does,
with no configuration.

**The topic in a pipeline.** CI checks the root out detached at the pipeline's
commit, so the branch comes from the environment:

| Platform | Pipeline | Variable |
|---|---|---|
| GitLab | merge request | `CI_MERGE_REQUEST_SOURCE_BRANCH_NAME` |
| GitLab | branch | `CI_COMMIT_BRANCH` (unset for tags) |
| GitHub | pull request | `GITHUB_HEAD_REF` |
| GitHub | push | `GITHUB_REF_NAME`, when `GITHUB_REF_TYPE` is `branch` |

The default branch is `CI_DEFAULT_BRANCH` on GitLab and the event payload's
`repository.default_branch` on GitHub. The pipeline's branch applies only to a
workspace whose root is the pipeline's own checkout — `HEAD` at `CI_COMMIT_SHA`
or `GITHUB_SHA` — so any other workspace a job builds is resolved as usual.

**Matching.** Every slot whose remote has a branch of the topic's name, and
that the root does not override, is taken from that branch's tip — detached
and read-only, like every CI checkout. Resolution already lists every
repository's refs, so matching costs no extra round trip.

**The merge gate: `git scale check`.** It fails while any slot resolves from a
topic branch rather than a revision written in a config — what a merge would
ship is then not what the pipeline tested — and while any config pins a
[build](dependencies.md#build), which is for trying out, never for shipping. It needs no history, so it runs on
shallow checkouts. In a merge request pipeline it applies only when the target
is a branch the root pins; a branch pipeline is always checked.

```yaml
gitscale-check:
  script: gitscale check
```

The message names the slot, the branch and its repository, the pin that would
ship, and the fix for each case:

```
Error: imports/d was taken from branch feat/x, not from v1-2026.10.01 pinned in imports/b/.gitscale.toml.
  This pipeline tested imports/d at feat/x (4f2a9c1), so merging now would ship a pin that was not tested.
  - imports/d's change not merged yet: merge it first, then run git upgrade --commit here and push.
  - already merged and pinned: delete branch feat/x in git@github.com:org/d.git, then rerun this pipeline.
```

**Artefacts on a topic.** A topic slot is its sources, also in a workspace that
takes the repository as an [artefact](artefacts.md#an-artefact-on-a-topic):
the change is tested as it is, locally and in CI alike, never the release
before it.

## Branch flows

Nothing above names `main`. Feature branches merged straight to the default
branch, a staging branch in between, GitFlow's `develop` and release branches
all work unchanged, because each step asks a question no flow changes: a topic
is any branch the root does not pin; promotion asks only about tags; the gate
only whether anything resolves from a branch.

**Long-lived branches are topics or pins — you choose.** Left out of
`[branches] pinned`, the root's `staging` matches every repository with a
`staging` branch, which is what a staging integration build wants. Listed in
it, `staging` builds from pins like `main`.

**Tag naming expresses the policy.** Where tags are cut decides only which tag
`upgrade` picks:

| How a team tags | What `upgrade` pins |
|---|---|
| Releases on `staging` (`v1-2026.10.01`) | Those tags, as with tags on the default branch |
| Pre-releases on `staging` (`v1-2026.10.01-rc1`), releases on `main` | An rc only where the current pin is already a pre-release |

## Parallel topics

Two topics at once are two worktrees of the root:

```
git topic start --worktree feat/other     # beside the clone: ../app-feat-other
```

or, in a [bare clone](#worktree-layout), every `git topic start` adds one. Its
checkouts are worktrees of the same [stores](stores.md), so nothing is
downloaded again. Git checks a branch out in one worktree at a time, so each
root worktree is on its own topic, and every topic's branches are visible from
all of them. `git topic list` shows them all; `git topic finish` removes a
finished one's worktree.

## Things to know

- **Matching is by name only.** An unrelated `fix-login` branch in another
  repository joins the topic, in CI and on placement. Use topic names that
  carry a ticket ID; `ls` notes a slot that joined from its remote as `topic,
  from remote` until the first placement puts it on a local branch, so a
  surprise match shows.
- **The gate protects only when it is required** on every repository that
  consumes others.
- **Content detection can be inconclusive** when a later commit rewrote the
  same lines; `upgrade` says so and never guesses.
- **A busy long-lived branch may never be wholly in a tag**: by the time
  `staging` is tagged, new commits have landed on it. `git upgrade <dir>` is the
  way out.

---

[← 2.5 Stores, placement and the CI cache](stores.md) · [Contents](README.md) · [Next → 2.7 Hooks](hooks.md)

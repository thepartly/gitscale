# 2.6 Topics: one change across several repositories

- [The idea](#the-idea)
- [A change across three layers](#a-change-across-three-layers)
- [Entering, joining and leaving a topic](#entering-joining-and-leaving-a-topic)
  - [Which branches are not topics](#which-branches-are-not-topics)
  - [Where each checkout goes](#where-each-checkout-goes)
  - [`gitscale develop`](#gitscale-develop)
  - [A new branch from a topic](#a-new-branch-from-a-topic)
- [Inside a topic](#inside-a-topic)
- [Promotion: `gitscale upgrade`](#promotion-gitscale-upgrade)
- [Raising a dependency: `gitscale upgrade <dir>`](#raising-a-dependency-gitscale-upgrade-dir)
- [Writing what resolution selected: `upgrade --resolved`](#writing-what-resolution-selected-upgrade---resolved)
- [Topics in CI](#topics-in-ci)
- [Branch flows](#branch-flows)
- [Parallel topics](#parallel-topics)
- [Things to know](#things-to-know)

## The idea

A change that spans several repositories is a **topic**: one branch name, the
same in the root and in every repository the change touches. **The root's
current branch is the topic**, so nothing about a topic is stored apart from
the branches themselves, and `git switch` on the root is how one is entered
and left.

Inside a topic, GitScale checks out the topic's branches instead of the
revisions the configs pin, and **no `.gitscale.toml` is edited** to test the
change. A CI pipeline on the branch does the same. Pins change only once a
layer is merged and released, and then `gitscale upgrade` writes the new tag
into the configs that ask for it, bottom layer first, until the root merges.

The one rule behind every detail below: **no surprises between local and
CI.** A workspace on a topic and a pipeline on the same branch resolve alike.

## A change across three layers

The root pins B at `v2026.09.30`; B pins D at `v2026.09.28`. The change touches
D and B.

```
$ git switch -c feat/price-cache
$ gitscale develop imports/d imports/b
imports/d on feat/price-cache, from v2026.09.28 (6be5fd3)
imports/b on feat/price-cache, from v2026.09.30 (1498d55)
…edit, build and test in the workspace…
$ gitscale commit -m "price cache"
$ gitscale push
```

D is merged and its pipeline tags `v2026.10.01`:

```
$ gitscale upgrade --commit
imports/b  not tagged yet
imports/d  promoted → v2026.10.01
  imports/b/.gitscale.toml   libs/d  v2026.09.28 → v2026.10.01
  imports/d  left the topic: imports/d at v2026.10.01
  commit  imports/b/.gitscale.toml: pin imports/d v2026.10.01
next to merge: imports/b
$ gitscale push
```

B is merged and tagged; `gitscale upgrade` bumps the root's pin of B; the root
merges, and the topic is finished.

## Entering, joining and leaving a topic

| Command | Does |
|---|---|
| `git switch -c <topic>` | Start a topic. Every checkout stays where it is until it is developed |
| `gitscale develop <dir>...` | Put checkouts on the topic, writable, from the commit each is at |
| `git switch <topic>` | Join a topic: every checkout whose store or remote has the branch goes onto it |
| `git switch <pinned branch>` | Leave the topic: every checkout back at the revision its configs pin |
| `gitscale develop --stop <dir>...` | Take checkouts off the topic, their topic branches deleted |

With the [git hook](hooks.md#git-hooks) installed, a `git switch` of the root
moves the checkouts itself; without it, run `gitscale pull` after the switch.

### Which branches are not topics

A branch the root **pins** is not a topic: on it, every checkout is at its pin.
By default the root pins only its remote's default branch — `main` and `master`
both, when there is no remote to ask. `[develop] pinned` names the pinned
branches instead:

```toml
[develop]
pinned = ["main", "staging", "release/*"]
```

A written list is exactly what is pinned: the default branch is not implied,
and `pinned = []` makes every branch a topic. A root on no branch at all has no
topic either.

### Where each checkout goes

On a topic, every `pull` places each checkout by the first of:

1. **A local branch of the topic** in its store: on it, writable. This is a
   checkout someone developed — here, or in another worktree of the root.
2. **The remote's branch of the topic**: on a local branch tracking it,
   writable. This is how a colleague's `feat/price-cache` in E is used, and it
   is what CI does.
3. **Neither**: detached at its pin, read-only.

A checkout with uncommitted changes is never moved: the entry fails, the rest
carry on, and `pull` exits non-zero. Leaving a topic works the same way, so a
dirty checkout stays on the topic branch until its changes are committed or
discarded. Commits are never lost by a move: they stay on the topic branch in
the store, and `git switch` back brings them out again.

A slot the root holds with `override = true` never joins a topic, locally or
in CI. A slot whose repository has a checkout per major develops each major
on its own branch — see [two majors](#inside-a-topic).

### `gitscale develop`

```
gitscale develop imports/d
gitscale develop imports/b/libs/d     # the same checkout, named by B's link to it
gitscale develop --stop imports/d
```

`develop` puts each named checkout on a branch of the topic's name, starting
from the commit it is at — its pin — and makes it writable. It is the only way
a checkout gets onto a topic branch that does not exist yet; nothing is
inferred from edits.

- An **artefact** checkout becomes a worktree of its source at the same commit,
  in place; see [artefacts → on a topic](artefacts.md#an-artefact-on-a-topic).
- An entry the root **overrides** is refused: `imports/d is held by override in
  .gitscale.toml; remove the override first`.
- A slot [held at its pin](#inside-a-topic) is refused.
- Off a topic, `develop` says to create a branch first: `git switch -c
  <topic>`.

`--stop` takes a checkout back to its pin and deletes its topic branch. It
refuses while the remote still has the branch — `pull` would follow it again
— and while the branch holds work no remote has.

### A new branch from a topic

`git switch -c feat/y` while on topic `feat/x` carries the topic along: every
checkout developed on `feat/x` gets a `feat/y` branch at the same commit, so a
change can be split or renamed without redeveloping each repository.

```
$ git switch -c feat/price-cache-2
Switched to a new branch 'feat/price-cache-2'
  carry  imports/b → feat/price-cache-2
Pulling latest changes...
  ok    imports/b (on feat/price-cache-2)
  ok    imports/d
```

The carry happens in the pull the [git hook](hooks.md#git-hooks) runs after the
switch; without the hook, run `gitscale pull` next. GitScale tells this from git's own records — the new branch's reflog holds
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
— `[develop] pinned` naming it — keeps what it asks for at its pins: a slot
only it asks for, and everything below that, stays put, and status says
`pinned by imports/b`. Any other requester's view is unchanged.

**Two majors.** When a repository has a checkout per major, the highest major
develops on the topic's own branch and every other on `<topic>@v<major>` —
`feat/x@v1`, `feat/x@v0.4` — since one branch cannot say which major it means.
CI matches the same names.

**`commit` and `push` act on the topic.** `commit` commits, on the topic
branch, the root and every topic checkout that has changes. Off the topic it
commits only the root. A checkout with changes that is not on the topic is
skipped and says how to bring it in:

```
  skip  imports/e (not on topic feat/x; run gitscale develop imports/e)
```

`push` pushes the topic branch of the root and every topic checkout as `git
push -u origin <branch>`: the remote branch gets the same name and becomes the
upstream.

**`status` follows the topic.** The table starts with the topic and what may
merge next, and each topic row says where its change stands and what it waits
on — see [status → topics](status.md#topics):

```
topic feat/price-cache · next to merge: imports/d
    REPO        PATH   ARTEFACT   REF                EXPECTED           STATUS   RESOLUTION
✔   imports/b   -      -          feat/price-cache   feat/price-cache   ok       topic, waits on imports/d, not tagged yet
✔   imports/d   -      -          feat/price-cache   feat/price-cache   ok       topic, not tagged yet, implicit via imports/b
```

A topic branch cut from an older release than another repository now asks for
gets `behind v2026.09.30 wanted by imports/c: rebase it`. That needs history,
so it is worked out from the store, and never in CI.

## Promotion: `gitscale upgrade`

`gitscale upgrade` finds which topic slots have reached a release, writes those
releases into the topic's configs that ask for less, and takes the promoted
slots off the topic.

**No merge strategy is assumed.** A squash or a rebase rewrites commits, so
history cannot tell whether a branch was merged; content can. A slot is
promoted when **the newest calver tag on its pin's stream contains the
change**: `git merge-tree` writes what merging the branch into the tag would
give, and it is the tag's own tree. Nor is a branch flow assumed: only tags are
asked, whichever branch they were cut on. This runs in each slot's store, which
has the history; never in CI.

| State | Meaning | `upgrade` does |
|---|---|---|
| `no change yet` | The branch adds nothing to the pin | Nothing |
| `not tagged yet` | The newest release does not hold the change: not merged, or its tag pipeline has not run | Nothing |
| `tagged <tag>, no image yet` | Released, but consumed as an artefact and the tag's commit has no image yet | Nothing |
| `promoted → <tag>` | The tag holds the change | Edits the configs, then the slot leaves the topic |
| `cannot tell` | Merging conflicts: a later commit in the tag rewrote the same lines | Nothing; bump it explicitly with `gitscale upgrade <dir>` |
| `held (… uncommitted)` | Work no tag can hold | Nothing; commit or discard it first |

Commits on the branch that nobody pushed do not hold a slot back: once the tag
holds their content, they are what a squash left behind.

**What it edits.** For each promoted slot, every config of the topic — the
root's, and each topic checkout's working tree — that asks for it below the
new tag gets the tag written in, in the tag's own spelling. Requesters outside
the topic are left alone: highest-wins resolution already lifts them. An
override in a requester is reported, not changed. The file is edited in place,
so comments and key order survive.

**Leaving.** The slot's topic branch is deleted and it is checked out detached
at the tag, read-only; an artefact gets the tag's image back. If the topic
branch still exists on the slot's remote, `upgrade` warns: `pull` and CI keep
matching it by name until it is deleted — which is why merged topic branches
are expected to be deleted, a setting GitLab ("Delete source branch") and
GitHub ("Automatically delete head branches") both have.

**The merge order.** A topic repository is ready to merge when no topic slot it
asks for, directly or further down, is still unpromoted; the root merges last.
`upgrade` and `status` print the next ones.

| Option | Meaning |
|---|---|
| `--commit` | Commit each edited `.gitscale.toml` — that file alone, whatever else the repository has changed — as `pin <dir> <tag>` |
| `--dry-run` | Print the plan; change nothing |

Pushing is left to [`gitscale push`](#inside-a-topic): `gitscale upgrade
--commit && gitscale push`.

## Raising a dependency: `gitscale upgrade <dir>`

`gitscale upgrade imports/d` raises D wherever it is asked for, topic or not:

1. The new revision is the **newest release on D's stream**: the newest calver,
   or for semver the newest of the current major; `--major` crosses majors. A
   pre-release is picked only when the current pin is already a pre-release —
   the rule npm and Cargo follow.
2. **Every requester is edited**: the root directly, every other one after
   it is [developed](#gitscale-develop) — an artefact requester becomes a
   checkout of its source first, to edit its config.
3. A requester asking for no version (a branch) is reported, not changed. A
   requester the root overrides cannot be developed, and is reported. If D
   itself is overridden, nothing is edited.
4. With no topic and a repository other than the root to edit, `upgrade`
   creates the root's branch with `git switch -c` and develops the requesters
   on it: `upgrade/d-v2026.10.01`, or `upgrade/<date>` for several
   dependencies. `-c <branch>` names it.

```
$ gitscale upgrade imports/d
imports/d   v2026.09.28 → v2026.10.01
  topic upgrade/d-v2026.10.01 (created): imports/b, imports/c developed
  imports/b/.gitscale.toml   libs/d  v2026.09.28 → v2026.10.01
  imports/c/.gitscale.toml   libs/d  v2026.09.30 → v2026.10.01
next to merge: imports/b, imports/c
```

The cascade then runs as for any change: B and C merge and are tagged, and
`gitscale upgrade` bumps the root's pins of them.

## Writing what resolution selected: `upgrade --resolved`

`gitscale upgrade --resolved [<dir>...]` writes the revision resolution already
selected into the root's own entries, with no tag lookup — so the diff of
`.gitscale.toml` shows what the workspace is really built from.

| Root entry | Result |
|---|---|
| Has a revision, resolved to a different one | Replaced with the winner's text |
| Has a revision, resolved to the same | Unchanged |
| No revision | Unchanged: leaving it out is a choice to follow the dependencies |
| `override = true` | Unchanged |
| Implicit dependency | Never added |

On a topic it writes the pin, never the topic branch. When the winner is a
branch replacing a tag, the line says so, since that turns a fixed pin into a
moving one. It edits only the root's config, so it never creates a topic.

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

**The merge gate: `gitscale check`.** It fails while any slot resolves from a
topic branch rather than a revision written in a config — what a merge would
ship is then not what the pipeline tested. It needs no history, so it runs on
shallow checkouts. In a merge request pipeline it applies only when the target
is a branch the root pins; a branch pipeline is always checked.

```yaml
gitscale-check:
  script: gitscale check
```

The message names the slot, the branch and its repository, the pin that would
ship, and the fix for each case:

```
Error: imports/d was taken from branch feat/x, not from v2026.10.01 pinned in imports/b/.gitscale.toml.
  This pipeline tested imports/d at feat/x (4f2a9c1), so merging now would ship a pin that was not tested.
  - imports/d's change not merged yet: merge it first, then run gitscale upgrade here and push.
  - already merged and pinned: delete branch feat/x in git@github.com:org/d.git, then rerun this pipeline.
```

**Missing images.** A topic slot consumed as a `replace` artefact uses the
image of its branch tip's commit; with none — the producer does not publish
on branches, or its pipeline has not finished — its sources are checked out
at that same commit instead, locally and in CI alike. It never falls back to the pinned
tag, which would test without the change and pass the gate, and never uses an
older commit's image.

## Branch flows

Nothing above names `main`. Feature branches merged straight to the default
branch, a staging branch in between, GitFlow's `develop` and release branches
all work unchanged, because each step asks a question no flow changes: a topic
is any branch the root does not pin; promotion asks only about tags; the gate
only whether anything resolves from a branch.

**Long-lived branches are topics or pins — you choose.** Left out of
`[develop] pinned`, the root's `staging` matches every repository with a
`staging` branch, which is what a staging integration build wants. Listed in
it, `staging` builds from pins like `main`.

**Tag naming expresses the policy.** Where tags are cut decides only which tag
`upgrade` picks:

| How a team tags | What `upgrade` pins |
|---|---|
| Releases on `staging` (`v2026.10.01`) | Those tags, as with tags on the default branch |
| Pre-releases on `staging` (`v2026.10.01-rc1`), releases on `main` | An rc only where the current pin is already a pre-release |
| A separate stream on `staging` (`staging-2026.10.01`) | The stream the current pin uses: streams are compared only within themselves |

## Parallel topics

Two topics at once are two worktrees of the root:

```
git worktree add ../app-other -b feat/other
```

The git hook populates it; without the hook, run `gitscale pull` in it.

Its checkouts are worktrees of the same [stores](stores.md), so nothing is
downloaded again. Git checks a branch out in one worktree at a time, so each
root worktree is on its own topic, and every topic's branches are visible from
all of them.

## Things to know

- **Matching is by name only.** An unrelated `fix-login` branch in another
  repository joins the topic, in CI and on `pull`. Use topic names that carry a
  ticket ID; `status` notes a slot that joined from its remote as `topic, from
  remote` until the first `pull` puts it on a local branch, so a surprise match
  shows.
- **The gate protects only when it is required** on every repository that
  consumes others.
- **Content detection can be inconclusive** when a later commit rewrote the
  same lines; `upgrade` says so and never guesses.
- **A busy long-lived branch may never be wholly in a tag**: by the time
  `staging` is tagged, new commits have landed on it. `upgrade <dir>` is the way
  out.

---

[← 2.5 Stores, worktrees and the CI cache](stores.md) · [Contents](README.md) · [Next → 2.7 Hooks](hooks.md)

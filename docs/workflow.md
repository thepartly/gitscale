# 2.4 Everyday workflow

- [One-time setup](#one-time-setup)
- [A plain clone](#a-plain-clone)
- [A bare clone with worktrees](#a-bare-clone-with-worktrees)
- [Occasional tasks](#occasional-tasks)
- [CI on hosted runners](#ci-on-hosted-runners)
- [Choosing a command](#choosing-a-command)

Each step below is what you want, as a comment; the GitScale command, with
what it does after it; then the plain git that does the same, with `lacks:`
for what it misses and `note:` for anything else. Git commands GitScale does
not know — `status`, `add`, `commit`, `push`, `pull`, `log` — run across the
workspace with `git scale`: see [git commands](cli.md#git-commands-git-scale-git-command).
What GitScale does to the checkouts after a command moves them is
[placement](stores.md#placement).

## One-time setup

With GitScale [installed](overview.md#install):

```sh
# Install the hook and the man pages
git scale hook install --global --allow 'github.com/acme/*'
#   or via native git:
#       not available
#       note: without the hook, run git scale sync after every clone and switch, and git scale pull instead of git pull
```

See [git hooks](hooks.md#git-hooks).

## A plain clone

```sh
# Clone the workspace
git clone git@github.com:acme/app.git && cd app    # hook: stores in .git/gitscale/repos, children detached and read-only

# See the workspace
git scale ls    # every checkout: revision, how it was chosen, state
#   or via native git:
#       not available

# Start a topic
git topic start PROJ-12-price-cache    # children with this branch (here or on the remote) join; the rest stay at their pins
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
#       lacks: detached children; a moved branch revision, or a topic a colleague started in one, waits for the next git scale pull

# Work on a colleague's topic
git topic switch PROJ-9-colleague    # children with PROJ-9 branches join
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

# Promote core, once it is merged and tagged v1-2026.10.04
git upgrade --commit    # root's pin → v1-2026.10.04, core's topic branch deleted here and on its remote, core detached at the tag; ends with: to push: <repos>
#   or via native git:
#       edit the revision in every .gitscale.toml on the topic that asks for core, commit each
#       git -C imports/core push origin --delete PROJ-12-price-cache
#       git -C imports/core switch --detach v1-2026.10.04
#       git -C imports/core branch -D PROJ-12-price-cache
#       lacks: finding the tag, checking it holds the change and the remote branch's, finding every config that asks for core

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

## A bare clone with worktrees

Run from `app/`, the directory holding the bare repository.

```sh
# Clone the workspace
git clone --bare git@github.com:acme/app.git app/.git && cd app
#   or via native git:
#       the same, then once:
#       git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*' && git fetch
#       note: a bare clone fetches no new remote branches until told to; the first topic command does this itself

# Add the main worktree
git topic switch main    # worktree app/main, its checkouts from the stores in app/.git/gitscale/repos
#   or via native git:
#       git worktree add main main

# Start a topic in its own worktree
git topic start PROJ-13-retry    # worktree app/PROJ-13-retry from origin/main, same stores; prints: cd PROJ-13-retry
#   or via native git:
#       git fetch && git worktree add -b PROJ-13-retry PROJ-13-retry origin/main
#       note: with a [topic] prefix, you add it to the branch and drop it from the directory

# Join core to the topic
cd PROJ-13-retry
git topic join imports/core    # as in a plain clone
#   or via native git:
#       git -C imports/core switch -c PROJ-13-retry

# Edit, commit, push, promote: as in a plain clone

# Work on a colleague's topic
git topic switch PROJ-9-colleague    # its own worktree; children with PROJ-9 branches join; prints: cd ../PROJ-9-colleague
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

See [worktree layout](topics.md#worktree-layout) and
[stores](stores.md#a-root-with-worktrees).

## Occasional tasks

```sh
# Leave the topic
git topic leave imports/core    # back at the pin, topic branch deleted; inside it: git topic leave .
#   or via native git:
#       not available
#       note: needs the pin from resolution, and lacks the refusal when the branch holds unpushed work

# Raise a dependency to its newest release, everywhere it is asked for
git topic start upgrade-d
git topic join --dependants imports/d    # every requester asking for less
git upgrade imports/d    # inside it: git upgrade .
#   or via native git:
#       edit every .gitscale.toml that asks for d
#       lacks: finding the release and every requester

# Carry the topic's change one level up
git topic join --dependants    # the checkouts asking for what changed on the topic
#   or via native git:
#       not available

# Promote the topic's released slots
git upgrade    # from anywhere
#   or via native git:
#       as git upgrade --commit above

# See why a checkout has its revision
git explain imports/utils    # inside it: git explain .
#   or via native git:
#       not available

# See every checkout more than one repo asks for
git explain
#   or via native git:
#       not available

# Run a git command in some checkouts
git scale log --oneline -5 --for imports/core
#   or via native git:
#       git -C imports/core log --oneline -5

# Run a git command in every checkout
git scale log --oneline -1 --foreach    # detached ones included
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
git scale clean -fdx    # checkouts and links always kept
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

## CI on hosted runners

Runners have no hook, so a job places the workspace itself:

```yaml
build:
  script:
    - gitscale sync
    - make
merge-gate:
  rules: [{ if: '$CI_PIPELINE_SOURCE == "merge_request_event"' }]
  script: [gitscale check]   # gates only merges into pinned branches, read from the MR target
```

In CI every resolution asks the remotes, and a remote that cannot be reached
fails it: runners keep the build directory between jobs, and refs an earlier
job fetched would build a branch where it was then. See
[CI authentication](ci-authentication.md) and [the CI cache](stores.md#the-ci-cache).

## Choosing a command

| You want to | Use |
|---|---|
| Set up a workspace | `git clone <url>`, with a [git hook](hooks.md#git-hooks) installed; else `git clone` then `git scale sync` |
| Check out entries added to the config | `git scale sync` |
| See what changed upstream, safely | `git scale fetch` then `git scale ls` |
| Get up to date | `git scale pull` |
| Change a dependency | `git topic start <topic>`, then [`git topic join <dir>`](topics.md#git-topic-join--leave) |
| Work on two things at once | `git topic start --worktree`, or a bare clone — see [parallel topics](topics.md#parallel-topics) |
| Turn a real checkout back into a dependency link | `git scale sync` (`--force` if it has local work) |

---

[← 2.3 Recursive dependencies](recursive-dependencies.md) · [Contents](README.md) · [Next → 2.5 Stores, placement and the CI cache](stores.md)

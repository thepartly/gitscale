# Plan: `git topic finish` deletes merged remote branches

Status: proposed, not started.

- [What it gives](#what-it-gives)
- [Which branches go](#which-branches-go)
- [When](#when)
- [What it says](#what-it-says)
- [`--force`](#--force)
- [Code](#code)
- [Tests](#tests)
- [Docs](#docs)

## What it gives

A finished topic leaves nothing behind on its remotes that its default
branches already hold. `git upgrade` deletes a promoted repository's topic
branch; `git topic finish` deletes the rest: the root's own, which nothing
promotes, and those of checkouts joined to the topic and merged without a
release.

No option and no question: whether a branch can go is decided by its content,
with the test promotion uses, and a branch whose change is not held stays.

## Which branches go

For the root, and for each store finish deletes a local branch of the topic
in:

1. **Its remote has the branch.** The root's is `origin/<branch>`, fetched
   with the rest at the start of finish; a store's is asked with
   `git ls-remote`.
2. **Its default branch holds the branch's change** — the root's
   `origin/<default>`, a store's `origin/HEAD` — by the content test finish
   already applies to the root's own branch: squash and rebase merges count.

A branch that passes both is deleted with
`git push --force-with-lease=refs/heads/<branch>:<tip> <url> :refs/heads/<branch>`,
at the tip checked, so a push made since fails the deletion instead of being
lost.

A remote branch in a repository this workspace never joined is someone
else's, and left alone.

## When

After finish's own checks pass, and after its local work — the switch to the
default branch, the removed worktree, the deleted local branches. A finish
refused for any reason touches no remote.

A deletion that fails — no permission, a protected branch, a push since the
check, the remote unreachable — is a warning, and finish still succeeds: what
it deletes is held by the default branch, so nothing is lost by keeping it.

## What it says

```
switched to main
deleted PROJ-12 in ., imports/b
deleted origin/PROJ-12 in ., imports/b: merged
kept origin/PROJ-12 in imports/core: origin/HEAD does not hold it; git push https://github.com/org/core.git --delete PROJ-12 to drop it
warning: cannot delete origin/PROJ-12 in imports/d: protected branch; delete it on https://github.com/org/d.git
```

A branch already gone — deleted by `git upgrade`, or by the hosting service on
merge — says nothing.

## `--force`

`--force` abandons a topic. Its merged remote branches go by the same rule;
an unmerged one stays, named with the command that drops it. So a topic
finished unmerged still comes back from its remote branches with
`git topic switch`, as before; a merged one has nothing to come back to, its
change being on the default branch.

## Code

- `src/commands/upgrade.rs`: `RemoteBranch` and its `delete` move to a module
  of their own, `src/remote_branch.rs`, shared with finish; upgrade's use is
  unchanged.
- `src/commands/topic.rs`, `finish`: while checking step 4, record for the
  root and each store with a local topic branch the remote branch's tip, and
  whether `merged` holds it against the default branch; after the local
  steps, delete those that pass, report those that stay, and turn a failed
  deletion into a warning on `err`.

## Tests

In `topic`:

- after a squash merge of the root's branch, finish deletes `origin/<topic>`
  and says so;
- a joined checkout's branch merged into its default branch is deleted; one
  that is not stays, with the command to drop it;
- a branch `git upgrade` already deleted says nothing;
- `--force` on an unmerged topic keeps every unmerged remote branch, and
  `git topic switch` brings the topic back from them;
- a remote that refuses the deletion (a `pre-receive` hook) gives a warning,
  and finish exits 0 with its local work done;
- a push to the remote branch after the check, and before the deletion,
  fails the deletion, which is a warning, and the push survives;
- a refused finish (not merged, uncommitted changes) leaves every remote
  branch.

## Docs

- `topics.md`, *Starting, switching and finishing topics*: the remote
  branches, replacing "remote branches are never touched", and the output
  example; `--force`; coming back to a finished topic.
- `cli.md`, `git topic finish`.
- `src/skill.md`: finish deletes merged remote branches itself; deleting an
  unmerged one stays the user's to decide.

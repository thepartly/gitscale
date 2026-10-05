---
name: gitscale
description: Work in a GitScale workspace (a git repository with a .gitscale.toml at its top) or on a change that spans several of the repositories it checks out. Use for the state of dependency checkouts, starting and joining topics, committing and pushing across repositories with git scale, promoting merged layers with git upgrade, and git scale check failures in CI.
---
{{header}}

# GitScale

GitScale checks out the repositories a root `.gitscale.toml` declares, and their own dependencies, as one workspace. Each dependency is a **child**: a git worktree under the root, at the revision chosen from every config that **pins** it, usually a calver tag. A child is detached and read-only until it joins a topic. An `artefact` entry holds prebuilt files from a registry: `replace` (only the image) or `overlay` (the sources plus the image's untracked files).

The **topic** is the root's current branch. Children that join it go on a branch of the same name, and CI uses same-named branches, so no config changes until a layer is released. On a pinned branch (by default the root's default branch) there is no topic.

A child is never a workspace of its own: every command typed inside one acts on the whole workspace, and directory arguments are paths from the current directory (`.` inside a child).

Docs: https://github.com/thepartly/gitscale/blob/v{{version}}/docs/ — offline: `gitscale help <command>`.

## Commands

| Command | Does | Docs |
|---|---|---|
| `git scale ls` | Every child: revision, artefact, topic state | status.md |
| `git explain <dir>` | Every request behind a child's revision, and which won | status.md |
| `git topic` | Print the topic of the checkout you are in; exits 1 off a topic | topics.md |
| `git topic start <name>` · `switch <name>` | Begin a topic from the default branch, or go to an existing one | topics.md |
| `git topic join <dir>` · `leave <dir>` | Put a child on the topic, writable; take it back to its pin | topics.md |
| `git topic status` | The current topic: what each joined repository still needs, what to merge next | topics.md |
| `git topic list` · `finish` | Every topic in progress; end a merged one | topics.md |
| `git scale <git command>` | Run git in the root and every child on the topic, dependencies first | workflow.md |
| `git scale pull` | Pull, then bring every child up to date, detached ones included | workflow.md |
| `git scale sync` | Put every child where resolution says, against the remotes now | workflow.md |
| `git upgrade [--commit]` | Pin newly tagged layers into the topic's configs | topics.md |
| `git upgrade <dir>... [--major]` | Raise a dependency to its newest release in every config | topics.md |
| `git upgrade --resolved` | Write resolved revisions into the root config | topics.md |
| `git scale check` | CI gate: fails while any child comes from a branch | topics.md |
| `git scale artefact show` | Each artefact entry: its image, the commit it needs, whether that commit is published, what is installed | artefacts.md |
| `git scale clean` · `gc` · `require` · `unrequire` · `hook` · `skill` | Housekeeping | cli.md |

Add `--dry-run` to any `git upgrade` to see the plan first.

## Making a multi-repo change

1. `git scale ls` to find the children the change touches.
2. `git topic start <ticket>-<slug>`. Put the ticket ID in the name: CI matches branches by name.
3. `git topic join <dir>` for each child to change. Edits already made in it come along.
4. Edit, build and test in the workspace. `git scale status` and `git scale diff` show every repository on the topic.
5. `git scale add -A`, `git scale commit -m "<message>"`, then `git scale push`: dependencies first, the root last, upstreams set on the first push.
6. Open one merge request per repository, in the order `git topic status` prints, lowest layer first.
7. When a layer has merged and its pipeline has tagged it: `git upgrade --commit`, then push what it lists. Repeat 6 and 7 until the root merges, then `git topic finish`.

For a second change in parallel in a bare clone with worktrees, `git topic start` adds a worktree for it; its children come from the same stores, with no download.

## Rules

- Never edit a pin to test a change. To change a child, `git topic join` it; never commit in a detached child.
- `git scale <git command>` runs only where the topic is. A child with changes that is not on it gets a line saying so and the `join` command that brings it in; run that rather than committing by hand.
- `git scale pull` is the one forwarded command that brings everything up to date from the remotes. Others fetch only what the workspace lacks, and only when they moved a child.
- A child whose move is blocked by uncommitted changes stays where it is, and the command exits non-zero. Commit, stash or discard, then run `git scale sync`.
- `git topic join` refuses off a topic (start one first), on an entry with `override = true` in the root config, and below a repository that pins the topic. Ask before removing an override or editing a `pinned` list.
- `pinned by <dir>` in `git scale ls`: a requester keeps that child at its pin, though its remote has the topic branch.
- `git scale check` failing: a child still comes from a branch. Merge it, run `git upgrade --commit` and push; if it is merged and its branch still exists, ask to delete the branch.
- A placement failing on a missing image: `git scale artefact show` names the commit with no image. Its producer's pipeline has not published it yet; wait, or ask the user.
- `not tagged yet` from `git upgrade`: the tag pipeline has not finished. Wait; never pin a branch instead.
- Ask the user before merging, deleting a remote branch, `git topic finish --force`, or editing `.gitscale.toml` beyond what `git upgrade` writes.

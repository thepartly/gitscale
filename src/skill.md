---
name: gitscale
description: Work in a GitScale workspace (a git repository with a .gitscale.toml at its top) or on a change that spans several of the repositories it checks out. Use for the state of dependency checkouts, starting and joining topics, committing and pushing across repositories with git scale, promoting merged layers with git upgrade, and git scale check failures in CI.
---
{{header}}

# GitScale

GitScale checks out the repositories a root `.gitscale.toml` declares, and their own dependencies, as one workspace. Each dependency is a **child**: a git worktree under the root, at the revision chosen from every config that **pins** it, usually a calver tag. A child is detached and read-only until it joins a topic. A child arrives as its sources or as an **artefact**: the prebuilt image of its release from a registry, with none of its git history. Each workspace chooses with `git scale prefer`; a child whose sources it cannot read arrives as an artefact, and a child on the topic is always its sources.

The **topic** is the root's current branch. Children that join it go on a branch of the same name, and CI uses same-named branches, so no config changes until a layer is released. On a pinned branch (by default the root's default branch) there is no topic.

A child is never a workspace of its own: every command typed inside one acts on the whole workspace, and directory arguments are paths from the current directory (`.` inside a child).

Docs: https://github.com/thepartly/gitscale/blob/v{{version}}/docs/ — offline: `gitscale help <command>`.

## Commands

| Command | Does | Docs |
|---|---|---|
| `git scale ls` | Every child: revision, form, topic state | status.md |
| `git explain <dir>` | Every request behind a child's revision, and which won | status.md |
| `git topic` | Print the topic of the checkout you are in; exits 1 off a topic | topics.md |
| `git topic start <name>` · `switch <name>` | Begin a topic from the default branch, or go to an existing one | topics.md |
| `git topic join <dir>` · `leave <dir>` | Put a child on the topic, writable; take it back to its pin | topics.md |
| `git topic join --dependants [<dir>]` | Join the children that ask for the topic's changes, one level up; with `<dir>`, those asking for less than its newest release | topics.md |
| `git topic status` | The current topic: what each joined repository still needs, what to merge next | topics.md |
| `git topic list` · `finish` | Every topic in progress; end a merged one | topics.md |
| `git scale <git command>` | Run git in the root and every child on the topic, dependencies first | workflow.md |
| `git scale pull` | Pull, then bring every child up to date, detached ones included | workflow.md |
| `git scale sync` | Put every child where resolution says, against the remotes now | stores.md |
| `git upgrade [--commit]` | Pin newly tagged layers into the topic's configs; their topic branches are deleted on their remotes | topics.md |
| `git upgrade <dir>... [--major]` | Raise a dependency to its newest release in the topic's configs; requesters off the topic are named | topics.md |
| `git scale check` | CI gate: fails while any child comes from a branch | topics.md |
| `git scale prefer --artefact <dir>...` | Take children as the images of their releases here; `--source` to go back. `git scale pull` applies it | artefacts.md |
| `git scale hash [<dir>...]` | The source hash: the tag a repository's image carries | artefacts.md |
| `git scale artefact show` | Each child taken as an artefact: its release, whether it is published, what is installed | artefacts.md |
| `gitscale artefact publish [--reuse] [<release>]` | In a producer's pipeline, after its build: publish the image of the sources checked out under their source hash; with a release name, tag the image with it; the pipeline tags the commit after. `--reuse` releases a squash merge without rebuilding | artefacts.md |
| `git scale require <dir> <url> [<revision>]` · `unrequire <dir>` | Add a dependency to the root config and place it; remove one | cli.md |
| `git scale clean` · `gc` · `hook` · `skill` | Housekeeping | cli.md |

`git upgrade` works on a topic only. Add `--dry-run` to see the plan first.

For output to read, use `git scale ls --format json`, `git topic status --format json` and `git scale hash --format json`. Without `--fetch` they report what is on this machine; `git scale ls --fetch` and `git topic status --fetch` ask the remotes first, for whether a layer is tagged yet. Every option of a command: `gitscale help <command>`.

## Making a multi-repo change

1. `git scale ls` to find the children the change touches.
2. `git topic start <ticket>-<slug>`. Put the ticket ID in the name: CI matches branches by name.
3. `git topic join <dir>` for each child to change. Edits already made in it come along.
4. Edit, build and test in the workspace. `git scale status` and `git scale diff` show every repository on the topic.
5. `git scale add -A`, `git scale commit -m "<message>"`, then `git scale push`: dependencies first, the root last, upstreams set on the first push.
6. Open one merge request per repository, in the order `git topic status` prints, lowest layer first.
7. When a layer has merged and its pipeline has tagged it: `git upgrade --commit`, then push what it lists. `git topic join --dependants` joins the next layer up when it needs the change too. Repeat 6 and 7 until the root merges, then `git topic finish`.

For a second change in parallel in a bare clone with worktrees, `git topic start` adds a worktree for it; its children come from the same stores, with no download.

## Rules

- Never edit a pin to test a change. To change a child, `git topic join` it; never commit in a detached child.
- `git scale <git command>` runs only where the topic is. A child with changes that is not on it gets a line saying so and the `join` command that brings it in; run that rather than committing by hand.
- `git scale pull` is the one forwarded command that brings everything up to date from the remotes. Others fetch only what the workspace lacks, and only when they moved a child.
- A child whose move is blocked by uncommitted changes stays where it is, and the command exits non-zero. Commit, stash or discard, then run `git scale sync`.
- `git topic join` refuses off a topic (start one first), on an entry with `override = true` in the root config, and below a repository that pins the topic. Ask before removing an override or editing a `pinned` list.
- `pinned by <dir>` in `git scale ls`: a requester keeps that child at its pin, though its remote has the topic branch.
- `git scale check` failing: a child still comes from a branch. Merge it, run `git upgrade --commit`, which also deletes its topic branch, and push.
- `git upgrade` failing on a remote branch that holds changes its release does not: someone pushed after the merge. Ask the user; never delete the branch yourself.
- A placement failing on a missing image: `git scale artefact show` names the release with no image. Ask the user whether one is coming, or take its sources with `git scale prefer --source <dir>`.
- A `hash:<source hash>` revision pins one build, to try it before it is released. `git scale check` refuses it; set it back to a release by hand before merging.
- `no tag` from `git upgrade`: no release holds the change. Find out from the user whether one is coming; never pin a branch instead.
- Ask the user before merging, deleting a remote branch by hand, `git topic finish --force`, `git scale sync --force` (it removes checkouts that hold work), `git scale clean -f`, or editing `.gitscale.toml` beyond what `git upgrade` and `git scale require` write.

---
name: gitscale
description: Work in a GitScale workspace (a git repository with a .gitscale.toml at its top) or on a change that spans several of the repositories it checks out. Use for the state of dependency checkouts, developing a dependency on the root's branch, committing and pushing across repositories, promoting merged layers with gitscale upgrade, and gitscale check failures in CI.
---
{{header}}

# GitScale

GitScale checks out the repositories a root `.gitscale.toml` declares, and their own dependencies, as one workspace. Each dependency is a **child**: a git worktree under the root, at the revision chosen from every config that **pins** it, usually a calver tag. A child is detached and read-only until you develop it. An `artefact` entry holds prebuilt files from a registry: `replace` (only the image) or `overlay` (the sources plus the image's untracked files).

The **topic** is the root's current branch. Children you develop go on a branch of the same name, and CI uses same-named branches, so no config changes until a layer is released. On a pinned branch (by default the root's default branch) there is no topic.

Docs: https://github.com/thepartly/gitscale/blob/v{{version}}/docs/ — offline: `gitscale <command> --help`.

## Commands

| Command | Does | Docs |
|---|---|---|
| `gitscale status [--why <dir>]` | Every child: revision, artefact, topic state, merge order | status.md |
| `gitscale pull` · `fetch` · `sync` | Create and update children; they follow the root's branch | workflow.md |
| `git switch -c <topic>` on the root | Start a topic; `git switch <pinned branch>` leaves it | topics.md |
| `gitscale develop <dir>` | Put a child on the topic, writable | topics.md |
| `gitscale develop --stop <dir>` | Take a child off the topic, back to its pin | topics.md |
| `gitscale commit -m <msg>` · `push` | Commit or push the root and every child on the topic | workflow.md |
| `gitscale upgrade [--commit]` | Pin newly tagged layers into the topic's configs | topics.md |
| `gitscale upgrade <dir>... [--major]` | Raise a dependency to its newest release in every config | topics.md |
| `gitscale upgrade --resolved` | Write resolved revisions into the root config | topics.md |
| `gitscale check` | CI gate: fails while any child comes from a branch | topics.md |
| `gitscale artefact show` | Each artefact entry: its image, the commit it needs, whether that commit is published, what is installed | artefacts.md |
| `gitscale artefact list` | The commits each artefact entry has images for | artefacts.md |
| `clean` · `add` · `remove` · `hook` · `skill` · `artefact publish` | Housekeeping and producers | cli.md |

Add `--dry-run` to any `upgrade` to see the plan first.

## Making a multi-repo change

1. `gitscale status` to find the children the change touches.
2. On the root: `git switch -c <ticket>-<slug>`. Put the ticket ID in the name: CI matches branches by name. The hook moves the children; without it, run `gitscale pull`.
3. `gitscale develop <dir>` for each child to change. Edits already made in it come along.
4. Edit, build and test in the workspace.
5. `gitscale commit -m "<message>"`, then `gitscale push`.
6. Open one merge request per repository, in the merge order `gitscale status` prints, lowest layer first.
7. When a layer has merged and its pipeline has tagged it: `gitscale upgrade --commit && gitscale push`. Repeat 6 and 7 until the root merges.

For a second change in parallel, add a worktree of the root: `git worktree add -b <topic> ../<dir>`. Its children come from the same stores, with no download.

## Rules

- Never edit a pin to test a change. To change a child, `develop` it; never commit in a detached child.
- `commit` skips children with changes that are off the topic and prints the `develop` command that brings them in. Run it rather than committing by hand.
- A child whose move is blocked by uncommitted changes stays where it is, and `pull` (or `git switch`, through the hook) exits non-zero. Commit, stash or discard, then run `gitscale pull`.
- `develop` refuses on a detached root or a pinned branch (create a topic branch first), on an entry with `override = true` in the root config, and below a repository that pins the topic. Ask before removing an override or editing a `pinned` list.
- `pinned by <dir>` in status: a requester keeps that child at its pin, though its remote has the topic branch.
- `gitscale check` failing: a child still comes from a branch. Merge it, run `upgrade` and push; if it is merged and its branch still exists, ask to delete the branch.
- `pull` failing on a missing image: `gitscale artefact show` names the commit with no image. Its producer's pipeline has not published it yet; wait, or ask the user.
- `not tagged yet` from `upgrade`: the tag pipeline has not finished. Wait; never pin a branch instead.
- Ask the user before merging, deleting a remote branch, or editing `.gitscale.toml` beyond what `upgrade` writes.

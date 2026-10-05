# Changelog

## Unreleased

GitScale is now a set of git commands: `git scale`, `git topic`, `git upgrade`
and `git explain`. Any command GitScale does not know is a git command, run in
the root and every checkout on the topic — `git scale status`, `git scale
commit`, `git scale push`, `git scale pull`. See the
[command line reference](docs/cli.md) and [the workflows](docs/workflow.md).

There are no aliases for the old names: an alias would stop that name from
reaching git.

| Old | New |
|---|---|
| `gitscale status` | `git scale ls` |
| `gitscale status --why [DIR]` | `git explain [DIR]` |
| `gitscale pull [NAMES]` | `git scale pull`, or `git scale sync [DIR]` |
| `gitscale sync [--force]` | `git scale sync [--force]`; it no longer pushes |
| `gitscale push [NAMES]` | `git scale push [--for DIR]` |
| `gitscale fetch [NAMES]` | `git scale fetch` |
| `gitscale commit -m MSG [NAMES]` | `git scale add -A [--for DIR] && git scale commit -m MSG [--for DIR]` |
| `gitscale add DIR URL REV` | `git scale require DIR URL [REV]` |
| `gitscale remove DIR` | `git scale unrequire DIR` |
| `gitscale clean -f` | `git scale clean -fdx` |
| `gitscale clean --gc` | `git scale gc` |
| `gitscale develop DIR` | `git topic join DIR` |
| `gitscale develop --stop DIR` | `git topic leave DIR` |
| `gitscale upgrade` | `git upgrade` |
| CI: `gitscale pull` | `gitscale sync` |

Also new:

- `git topic start | switch | status | list | finish`, for plain clones and
  for a bare clone with a worktree per topic, with an optional
  `[topic] prefix`.
- Directory arguments are paths from the current directory, `.` inside a
  checkout; `-C` acts as the current directory, as git's own does.
- A checkout GitScale made is never a workspace of its own: a command typed
  inside one acts on the whole workspace, and a git hook firing in one places
  the workspace.
- `git scale require` and `unrequire` edit `.gitscale.toml` in place and place
  the workspace.
- Colour and icons only on a terminal, set with `--color`, off with `NO_COLOR`.
- In CI, resolution always asks the remotes: a remote that cannot be reached
  fails it.
- `git scale hook install` writes man pages, so `git scale --help` works.

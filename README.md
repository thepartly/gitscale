# GitScale

Manage multiple sub-repositories from a single config file. An alternative to
git submodules — simpler, with read-only checkouts at their pins, changes
across repositories on one branch name, and prebuilt output published to an
OCI registry.

📖 **[Full documentation](docs/README.md)**

## Install

```
cargo install gitscale
```

Requires a stable Rust toolchain. To build from a checkout instead:

```
git clone https://github.com/thepartly/gitscale.git
cd gitscale && cargo install --path .
```

This installs `gitscale` and `git-scale`, so `git scale <command>` works too.

## Quick start

Create a `.gitscale.toml` in your project root:

```toml
[repos]
"imports/core"  = { url = "https://github.com/org/core.git", revision = "main" }
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
```

Add the checkout directory to `.gitignore` — the workspace repo tracks the
selection in `.gitscale.toml`, not the checkouts themselves:

```gitignore
/imports/
```

Then check everything out:

```
gitscale pull
```

Check status:

```
gitscale status
```

```
    REPO             PATH   ARTEFACT   REF       EXPECTED   STATUS    RESOLUTION
✔   imports/core     -      -          8c1d0e2   main       ok
⤷   imports/shared   ../s   -          8c1d0e2   main       symlink
✔   imports/utils    -      -          3f2a9c1   v2.3       ok        raised from v2.1 by imports/core, 2 requests
hint: gitscale status --why <dir> lists every request behind a revision
```

## Cloning a workspace

Everyone else just clones the repository. With a `--global` or `--system`
[git hook](docs/hooks.md#git-hooks) installed, the `post-checkout` git fires at
the end of the clone materialises every declared repository:

```
gitscale hook install --global --allow 'github.com/org/*'   # once per machine
git clone https://github.com/org/root.git
```

Without the hook, run `gitscale pull` in the clone.

## Changing several repositories at once

The root's branch is the *topic*. Put the checkouts you change on it, and a CI
pipeline on the branch takes them from their branches too — no config edits:

```
git switch -c feat/price-cache
gitscale develop imports/core imports/utils
gitscale commit -m "price cache" && gitscale push
```

Once each layer is merged and tagged, `gitscale upgrade` writes the new tags
into the configs that ask for them. See [topics](docs/topics.md).

## What it gives you

- **One human-readable config** for every dependency, instead of `.gitmodules`
  plus gitlink entries.
- **Read-only checkouts at their pins**, writable only once developed on the
  topic.
- **Artefacts** — the build output the repository's own pipeline published with
  `gitscale artefact publish`, one OCI image per commit, pulled from GitLab's
  registry, GHCR or any other: instead of a checkout (`replace`) or laid over
  one (`overlay`).
- **Transitive dependencies resolved as one graph**: every repository asks for
  what it needs, the highest version wins, each major is checked out once and
  linked into every dependant, and dependencies the root never names are
  brought in under a configurable directory (`imports/` by default) from an
  allowlist. `status --why` shows how each
  revision was chosen.
- **Every checkout a git worktree** of one bare store per dependency, inside the
  root's own `.git`: a second worktree of the root costs nothing over the wire,
  and deleting the root leaves nothing behind. In CI, a per-user cache means
  the next job on the same runner downloads nothing.
- **Git hooks** that make `git clone`, `git switch` and `git worktree add`
  materialise the whole workspace, with an allowlist controlling what may run.
- **CI credentials without pipeline setup** — inside a GitLab or GitHub job,
  entries on that same server are fetched with the job token, which never
  reaches `.git/config` or a command line.
- **An agent skill** — `gitscale skill install` teaches coding agents
  (Claude Code, Codex, Cursor and others) to carry a change across the
  workspace's repositories, and stays in step with the installed version.
- **One status table** covering ahead/behind, ref mismatch, dirty, stale and
  broken-link states, and where a topic stands, with JSON output.

## Documentation

| | |
|---|---|
| [Overview](docs/overview.md) | What GitScale is, and why |
| [Declaring dependencies](docs/dependencies.md) | Entries, artefacts, pinning |
| [Status](docs/status.md) | Every flag `gitscale status` prints |
| [Recursive dependencies](docs/recursive-dependencies.md) | Hoisting, symlink dedup, version mismatches |
| [Everyday workflow](docs/workflow.md) | `fetch`, `pull`, `push`, `sync`, `commit` |
| [Stores, worktrees and the CI cache](docs/stores.md) | Where checkouts come from, roots made of worktrees, the CI cache |
| [Topics](docs/topics.md) | One change across several repositories: `develop`, `upgrade`, `check` |
| [Hooks](docs/hooks.md) | `[hooks]` commands, and GitScale as a git hook |
| [CI authentication](docs/ci-authentication.md) | Job tokens on GitLab and GitHub |
| [Cleaning](docs/clean.md) | `gitscale clean`, what it always keeps, and `--gc` |
| [Artefacts](docs/artefacts.md) | Publishing build output to an OCI registry, and installing it |
| [The agent skill](docs/agents.md) | Teaching coding agents the GitScale workflow |
| [Configuration reference](docs/configuration.md) | Every table, key and environment variable |
| [Command line reference](docs/cli.md) | Every command, argument and flag |
| [Related tools](docs/related-tools.md) | Comparison with submodules, repo, west, vcstool and others |

## License

MIT

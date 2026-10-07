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

This installs `gitscale`, `git-scale`, `git-topic`, `git-upgrade` and
`git-explain`, so git runs `git scale`, `git topic`, `git upgrade` and
`git explain`. `gitscale` takes the same command line as `git scale`.

## Quick start

Create a `.gitscale.toml` in your project root:

```toml
[repos]
"imports/core"  = { url = "https://github.com/org/core.git", revision = "main" }
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
```

Then check everything out — GitScale keeps the checkouts out of the workspace
repository's `git status` itself:

```
git scale sync
```

See the workspace:

```
git scale ls
```

```
    REPO             PATH   AS         REF       EXPECTED   STATUS    RESOLUTION
✔   imports/core     -      source     8c1d0e2   main       ok
⤷   imports/shared   ../s   source     8c1d0e2   main       symlink
✔   imports/utils    -      source     3f2a9c1   v2.3       ok        raised from v2.1 by imports/core, 2 requests
hint: git explain <dir> lists every request behind a revision
```

## Cloning a workspace

Everyone else just clones the repository. With a `--global` or `--system`
[git hook](docs/hooks.md#git-hooks) installed, the `post-checkout` hook git fires at
the end of the clone materialises every declared repository:

```
git scale hook install --global --allow 'github.com/org/*'   # once per machine
git clone https://github.com/org/root.git
```

Without the hook, run `git scale sync` in the clone.

## Changing several repositories at once

The root's branch is the *topic*. Put the checkouts you change on it, and a CI
pipeline on the branch takes them from their branches too — no config edits:

```
git topic start PROJ-12-price-cache
git topic join imports/core
git scale add -A
git scale commit -m "PROJ-12 price cache"
git scale push                 # dependencies first, the root last
```

`git scale <git command>` runs git in the root and every checkout on the
topic. Once a layer is merged and tagged, `git upgrade --commit` writes the new
tag into the configs that ask for it; push that, and once the root is merged:

```
git topic finish               # back on main, every checkout at its pin
```

See [everyday workflow](docs/workflow.md) and [topics](docs/topics.md).

## Demo

[gitscale-demo](https://github.com/thepartly/gitscale-demo) is a small system
of six repositories — a frontend, two applications, a shared library, a
local-stack helper and the workspace root — with every dependency pinned to a
release. Clone the root and follow its
[`DEMO.md`](https://github.com/thepartly/gitscale-demo/blob/main/DEMO.md):

```
git clone https://github.com/thepartly/gitscale-demo.git && cd gitscale-demo
git scale ls
```

It walks through topics across repositories, releasing a shared library to
some of its users and not others, taking dependencies as artefacts instead of
sources, running the stack locally from any repository, and rendering the
deployment from what is pinned.

## Why

The problems GitScale is built for:

- **One config, not a mechanism per dependency.** A single human-readable TOML
  file instead of `.gitmodules` plus gitlink entries, an XML manifest, or a
  Python `DEPS` file. See [declaring dependencies](docs/dependencies.md).
- **Dependencies that cannot be edited by accident.** Every checkout sits at
  the exact commit its pin names, with the write bit stripped off every file,
  so an accidental edit fails loudly instead of drifting silently.
- **One change across several repositories, with no config edits.** The root's
  branch is the *topic*: `git topic join imports/core` puts that checkout on a
  branch of the same name, writable, and a CI pipeline on the branch takes every
  repository's branch of that name too. `git scale commit` and `git scale push`
  run in every repository on the topic, dependencies first. Once the layers are
  released, `git upgrade` writes the new tags in. See [topics](docs/topics.md).
- **Large prebuilt payloads.** `git scale prefer --artefact` takes a
  repository's build output instead of cloning it — datasets, generated
  clients, compiled assets — published by its own pipeline with
  `gitscale artefact publish` to an OCI registry (GitLab's, GHCR, or any other)
  for each release, and fetched with the CI job token. Each workspace chooses,
  per dependency; one that cannot read a repository's sources gets its
  artefact by itself. No `docker`, `oras` or cloud CLI is needed. See
  [artefacts](docs/artefacts.md).
- **Shared transitive dependencies checked out once.** When two repositories in
  the workspace both depend on a third, it is checked out once — at the highest
  version either asks for, one checkout per major — and symlinked into each
  dependant, rather than two silently divergent copies. A dependency the root
  never declares is brought in on its own, under `imports/` by default and only
  from an allowlist, and `git explain` shows how each revision was chosen. See
  [recursive dependencies](docs/recursive-dependencies.md).
- **A checkout that populates itself.** With GitScale installed as a git hook,
  `git clone`, `git checkout` and `git worktree add` materialise the whole
  workspace — no `--recursive` flag to remember, and an allowlist decides what
  may run. The other hooks you choose keep running — each repository's own,
  git-lfs's, and the ones a repository commits in `.githooks/`, with no setup
  per clone. See [hooks](docs/hooks.md).
- **Downloads paid for once per workspace.** Every dependency is one bare
  clone inside the root's own `.git`, and every checkout of it a worktree: a
  second worktree of the root costs nothing over the wire, and deleting the root
  leaves nothing behind. In CI, a per-user cache means the next job on the same
  runner downloads nothing. See [stores](docs/stores.md).
- **CI that works without pipeline surgery.** Inside a GitLab or GitHub job,
  entries hosted on that same server are fetched over HTTPS with the job token,
  and the token never reaches `.git/config` or a command line. See
  [CI authentication](docs/ci-authentication.md).
- **Coding agents that know the workflow.** `git scale skill install` teaches
  Claude Code, Codex, Cursor and others to carry a change across the
  workspace's repositories, and stays in step with the installed version. See
  [the agent skill](docs/agents.md).
- **One glance at the whole workspace.** `git scale ls` reports ahead/behind,
  ref mismatch, dirty, stale and broken-link states, and where a topic stands,
  for every repo in one table, with JSON for anything that wants to consume it.
  See [status](docs/status.md).

Deliberate limits: GitScale targets git only (plus its own artefact archives),
it is a separate binary rather than something shipped with git, and a topic is
nothing but branches of one name — no manifest of its own, no server. The
[design choices](docs/related-tools.md#design-choices) section covers the
reasoning.

## Documentation

| | |
|---|---|
| [Overview](docs/overview.md) | What GitScale is, and getting a workspace |
| [Declaring dependencies](docs/dependencies.md) | Entries, artefacts, pinning |
| [Status](docs/status.md) | Every flag `git scale ls` prints, and `git explain` |
| [Recursive dependencies](docs/recursive-dependencies.md) | Hoisting, symlink dedup, version mismatches |
| [Everyday workflow](docs/workflow.md) | `git scale <git command>`, plain and bare clones, CI on hosted runners |
| [Stores, placement and the CI cache](docs/stores.md) | Where checkouts come from, placement, roots made of worktrees, the CI cache |
| [Topics](docs/topics.md) | One change across several repositories: `git topic`, `git upgrade`, `git scale check` |
| [Hooks](docs/hooks.md) | `[hooks]` commands, and GitScale as a git hook |
| [CI authentication](docs/ci-authentication.md) | Job tokens on GitLab and GitHub |
| [Cleaning](docs/clean.md) | `git scale clean`, what it always keeps, and `gc` |
| [Artefacts](docs/artefacts.md) | Publishing build output to an OCI registry, and installing it |
| [The agent skill](docs/agents.md) | Teaching coding agents the GitScale workflow |
| [Configuration reference](docs/configuration.md) | Every table, key and environment variable |
| [Command line reference](docs/cli.md) | Every command, argument and flag |
| [Related tools](docs/related-tools.md) | Comparison with submodules, repo, west, vcstool and others |

## License

MIT

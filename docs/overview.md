# 1. Overview

- [What GitScale is](#what-gitscale-is)
- [Why](#why)
- [What it looks like](#what-it-looks-like)
- [Install](#install)
- [Getting a workspace](#getting-a-workspace)
- [Where to go next](#where-to-go-next)

## What GitScale is

GitScale assembles a workspace out of several git repositories. One
`.gitscale.toml` at the root of a project says which repositories belong to it,
where each one is checked out, and which revision each one is pinned to:

```toml
[repos]
"imports/core"  = { url = "https://github.com/org/core.git", revision = "main" }
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
```

`git scale sync` materialises all of it; `git scale ls` shows the state of
every checkout in one table; `git scale pull` brings everything up to date.
Each checkout is a git worktree with its repository's full history — the
workspace repository tracks only the *selection*, in `.gitscale.toml`.

It is closest in spirit to git submodules, and is meant to replace them where
submodules are awkward.

## Why

The problems GitScale is built for:

- **One config, not a mechanism per dependency.** A single human-readable TOML
  file instead of `.gitmodules` plus gitlink entries, an XML manifest, or a
  Python `DEPS` file.
- **Dependencies that cannot be edited by accident.** Every checkout sits at
  the exact commit its pin names, with the write bit stripped off every file,
  so an accidental edit fails loudly instead of drifting silently.
- **One change across several repositories, with no config edits.** The root's
  branch is the *topic*: `git topic join imports/core` puts that checkout on a
  branch of the same name, writable, and a CI pipeline on the branch takes every
  repository's branch of that name too. `git scale commit` and `git scale push`
  run in every repository on the topic, dependencies first. Once the layers are
  released, `git upgrade` writes the new tags in. See [topics](topics.md).
- **Large prebuilt payloads.** `git scale prefer --artefact` takes a
  repository's build output instead of cloning it — datasets, generated
  clients, compiled assets — published by its own pipeline to an OCI registry
  (GitLab's, GHCR, or any other) for each release, and fetched with the CI job
  token. A workspace that cannot read a repository's sources gets its
  artefact by itself. No `docker`, `oras` or cloud CLI is needed. See
  [artefacts](artefacts.md).
- **Shared transitive dependencies checked out once.** When two repositories in
  the workspace both depend on a third, it is checked out once — at the highest
  version either asks for, one checkout per major — and symlinked into each
  dependant, rather than two silently divergent copies. A dependency the root
  never declares is brought in on its own. See
  [recursive dependencies](recursive-dependencies.md).
- **A checkout that populates itself.** With GitScale installed as a git hook,
  `git clone`, `git checkout` and `git worktree add` materialise the whole
  workspace — no `--recursive` flag to remember. See [hooks](hooks.md).
- **Downloads paid for once per workspace.** Every dependency is one bare
  clone inside the root's own `.git`, and every checkout of it a worktree: a
  second worktree of the root costs nothing over the wire, and deleting the root
  leaves nothing behind. In CI, a per-user cache means the next job on the same
  runner downloads nothing. See [stores](stores.md).
- **CI that works without pipeline surgery.** Inside a GitLab or GitHub job,
  entries hosted on that same server are fetched over HTTPS with the job token,
  and the token never reaches `.git/config` or a command line. See
  [CI authentication](ci-authentication.md).
- **One glance at the whole workspace.** `git scale ls` reports ahead/behind,
  ref mismatch, dirty, stale and broken-link states, and where a topic stands, for every repo in
  one table, with JSON for anything that wants to consume it. See
  [status](status.md).

Deliberate limits: GitScale targets git only (plus its own artefact archives),
it is a separate binary rather than something shipped with git, and a topic is
nothing but branches of one name — no manifest of its own, no server. The
[design choices](related-tools.md#design-choices) section covers the reasoning.

## What it looks like

```
$ git scale ls
    REPO            PATH   AS         REF       EXPECTED   STATUS   RESOLUTION
✔   imports/core    -      source     3f2a9c1   main       ok
✔   imports/utils   -      source     8c1d0e2   v2.1.0     ok
✔   imports/d       -      source     6be5fd3   v1.4.0     ok       implicit via imports/core
```

## Install

```
cargo install gitscale
```

Requires a stable Rust toolchain. To build from a checkout instead:

```
git clone https://github.com/thepartly/gitscale.git
cd gitscale && cargo install --path .
```

Five binaries are installed: `gitscale`, `git-scale`, `git-topic`,
`git-upgrade` and `git-explain`, which let git dispatch `git scale`,
`git topic`, `git upgrade` and `git explain` to GitScale. `gitscale` takes the
same command line as `git scale` — `git scale ls` and `gitscale ls` are the
same command. See [the synopsis](cli.md#synopsis).

## Getting a workspace

Authoring one means writing a `.gitscale.toml`, gitignoring the checkout
directory, and running `git scale sync` — see
[declaring dependencies](dependencies.md).

Using one means `git clone`, and nothing else. A `--global` or `--system`
[git hook](hooks.md#git-hooks), installed once per machine, materialises every
declared repository at the end of the clone. Where no hook applies, `git clone`
followed by [`git scale sync`](workflow.md#placement) does the same work. A
bare clone with a worktree per topic works too — see
[everyday workflow](workflow.md#a-bare-clone-with-worktrees).

## Where to go next

- Setting up a workspace: [declaring dependencies](dependencies.md)
- Day-to-day commands: [everyday workflow](workflow.md)
- A change across repositories: [topics](topics.md)
- Every key in the config file: [configuration reference](configuration.md)

---

[← Contents](README.md) · [Contents](README.md) · [Next → 2.1 Declaring dependencies](dependencies.md)

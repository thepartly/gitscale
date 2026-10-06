# 1. Overview

- [What GitScale is](#what-gitscale-is)
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

`git clone` materialises all of it, with GitScale's
[git hook](hooks.md#git-hooks) installed; `git scale ls` shows the state of
every checkout in one table; `git scale pull` brings everything up to date.
Each checkout is a git worktree with its repository's full history — the
workspace repository tracks only the *selection*, in `.gitscale.toml`.

It is closest in spirit to git submodules, and is meant to replace them where
submodules are awkward. The problems it is built for are listed under
[why](../README.md#why) in the project README.

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

Authoring one means writing a `.gitscale.toml` and running `git scale sync` —
see [declaring dependencies](dependencies.md).

Using one means `git clone`, and nothing else. A `--global` or `--system`
[git hook](hooks.md#git-hooks), installed once per machine, materialises every
declared repository at the end of the clone. Where no hook applies, `git clone`
followed by [`git scale sync`](stores.md#placement) does the same work. A
bare clone with a worktree per topic works too — see
[everyday workflow](workflow.md#a-bare-clone-with-worktrees).

## Where to go next

- Setting up a workspace: [declaring dependencies](dependencies.md)
- Day-to-day commands: [everyday workflow](workflow.md)
- A change across repositories: [topics](topics.md)
- Every key in the config file: [configuration reference](configuration.md)

---

[← Contents](README.md) · [Contents](README.md) · [Next → 2.1 Declaring dependencies](dependencies.md)

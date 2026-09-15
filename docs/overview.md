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
"imports/core"  = { url = "https://github.com/org/core.git", revision = "main", mode = "readonly" }
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
```

`gitscale clone` materialises all of it; `gitscale status` shows the state of
every checkout in one table; `gitscale sync` brings everything back in line.
The sub-repositories stay ordinary git clones with their own history — the
workspace repository tracks only the *selection*, in `.gitscale.toml`.

It is closest in spirit to git submodules, and is meant to replace them where
submodules are awkward.

## Why

The problems GitScale is built for:

- **One config, not a mechanism per dependency.** A single human-readable TOML
  file instead of `.gitmodules` plus gitlink entries, an XML manifest, or a
  Python `DEPS` file.
- **Vendored code that cannot be edited by accident.** `mode = "readonly"`
  strips the write bit off every checked-out file, so an accidental edit fails
  loudly instead of drifting silently. See
  [checkout modes](dependencies.md#checkout-modes).
- **Large prebuilt payloads.** `mode = "artefact"` pulls a `tar.gz` from any
  S3-compatible bucket instead of cloning — datasets, generated clients, closed
  blobs. Signing and transfer are built in; no `aws` CLI or SDK is needed. See
  [artefact storage](dependencies.md#artefact-storage).
- **Shared transitive dependencies checked out once.** When two repositories in
  the workspace both depend on a third, it is checked out once at the root and
  symlinked into each dependant, and conflicting revision claims are an error
  rather than two silently divergent copies. See
  [recursive dependencies](recursive-dependencies.md).
- **A checkout that populates itself.** With GitScale installed as a git hook,
  `git clone`, `git checkout` and `git worktree add` materialise the whole
  workspace — no `--recursive` flag to remember. See [hooks](hooks.md).
- **Downloads paid for once per machine.** An object cache — on by default —
  means a second workspace, a second worktree, or the next CI job on the same
  runner costs nothing over the wire. See [the object cache](caching.md).
- **CI that works without pipeline surgery.** Inside a GitLab or GitHub job,
  entries hosted on that same server are fetched over HTTPS with the job token,
  and the token never reaches `.git/config` or a command line. See
  [CI authentication](ci-authentication.md).
- **One glance at the whole workspace.** `gitscale status` reports ahead/behind,
  detached, ref mismatch, dirty, stale and broken-link states for every repo in
  one table, with JSON for anything that wants to consume it. See
  [status](status.md).

Deliberate limits: GitScale targets git only (plus its own artefact archives),
it is a separate binary rather than something shipped with git, and it does not
try to manage a shared multi-repo branching lifecycle the way Google repo or
west do. The
[design choices](related-tools.md#design-choices) section covers the reasoning.

## What it looks like

```
$ gitscale status
      REPO             PATH   MODE        REF    EXPECTED   STATUS
✔     imports/core     -      readonly    main   main       ok
✔     imports/utils    -      readwrite   v2.1   v2.1       ok
⤷     imports/shared   ../s   readonly    main   main       symlink
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

Two binaries are installed: `gitscale`, and `git-scale`, which lets git dispatch
to it — so `git scale status` and `gitscale status` are the same command.

## Getting a workspace

Authoring one means writing a `.gitscale.toml`, gitignoring the checkout
directory, and running `gitscale clone` — see
[declaring dependencies](dependencies.md).

Using one means `git clone`, and nothing else. A `--global` or `--system`
[git hook](hooks.md#git-hooks), installed once per machine, materialises every
declared repository at the end of the clone, and
[`[cache] adopt_root`](caching.md#adopting-a-root-repository) puts the root
repository on the object cache while it is there. Where no hook applies,
[`gitscale clone <url>`](workflow.md#cloning-a-workspace-from-a-url) does the
same work explicitly.

## Where to go next

- Setting up a workspace: [declaring dependencies](dependencies.md)
- Day-to-day commands: [everyday workflow](workflow.md)
- Every key in the config file: [configuration reference](configuration.md)

---

[← Contents](README.md) · [Contents](README.md) · [Next → 2.1 Declaring dependencies](dependencies.md)

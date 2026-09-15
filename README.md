# GitScale

Manage multiple sub-repositories from a single config file. An alternative to
git submodules — simpler, with readonly enforcement, artefact mode, and cloud
storage for pre-built archives.

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
"imports/core"  = { url = "https://github.com/org/core.git", revision = "main", mode = "readonly" }
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
```

Add the checkout directory to `.gitignore` — the workspace repo tracks the
selection in `.gitscale.toml`, not the checkouts themselves:

```gitignore
/imports/
```

Then clone everything:

```
gitscale clone
```

Check status:

```
gitscale status
```

```
      REPO             PATH   MODE        REF    EXPECTED   STATUS
✔     imports/core     -      readonly    main   main       ok
✔     imports/utils    -      readwrite   v2.1   v2.1       ok
⤷     imports/shared   ../s   readonly    main   main       symlink
```

## Cloning a workspace

Everyone else just clones the repository. With a `--global` or `--system`
[git hook](docs/hooks.md#git-hooks) installed, the `post-checkout` git fires at
the end of the clone materialises every declared repository, and
[`[cache] adopt_root`](docs/caching.md#adopting-a-root-repository) puts the root
itself on the object cache:

```
git clone https://github.com/org/root.git
```

That needs the hook installed once per machine, and `adopt_root` in the
workspace config:

```
gitscale hook install --global --allow 'github.com/org/*'
```

```toml
# .gitscale.toml
[cache]
adopt_root = true
```

Where no hook is installed, `gitscale clone` takes the URL instead and does the
same work explicitly:

```
gitscale clone https://github.com/org/root.git
```

## What it gives you

- **One human-readable config** for every dependency, instead of `.gitmodules`
  plus gitlink entries.
- **Three checkout modes** — `readwrite`, `readonly` (write bits stripped on
  disk), and `artefact` (a prebuilt `tar.gz` pulled from any S3-compatible
  bucket).
- **Transitive dependencies deduped by symlink**, hoisted to the root, with
  conflicting revision claims reported as an error rather than silently
  divergent copies.
- **An object cache, on by default**, so a second workspace, a second worktree
  or the next CI job on the same runner costs nothing over the wire.
- **Git hooks** that make `git clone`, `git checkout` and `git worktree add`
  materialise the whole workspace, with an allowlist controlling what may run.
- **CI credentials without pipeline setup** — inside a GitLab or GitHub job,
  entries on that same server are fetched with the job token, which never
  reaches `.git/config` or a command line.
- **One status table** covering ahead/behind, detached, ref mismatch, dirty,
  stale and broken-link states, with JSON output.

## Documentation

| | |
|---|---|
| [Overview](docs/overview.md) | What GitScale is, and why |
| [Declaring dependencies](docs/dependencies.md) | Entries, checkout modes, pinning, artefact storage |
| [Status](docs/status.md) | Every flag `gitscale status` prints |
| [Recursive dependencies](docs/recursive-dependencies.md) | Hoisting, symlink dedup, version mismatches |
| [Everyday workflow](docs/workflow.md) | `clone`, `fetch`, `pull`, `push`, `sync`, `commit` |
| [The object cache](docs/caching.md) | Mirrors, snapshots, CI, `dissociate`, `adopt_root` |
| [Hooks](docs/hooks.md) | `[hooks]` commands, and GitScale as a git hook |
| [CI authentication](docs/ci-authentication.md) | Job tokens on GitLab and GitHub |
| [Cleaning](docs/clean.md) | `gitscale clean` and what it always keeps |
| [Configuration reference](docs/configuration.md) | Every table, key and environment variable |
| [Command line reference](docs/cli.md) | Every command, argument and flag |
| [Related tools](docs/related-tools.md) | Comparison with submodules, repo, west, vcstool and others |

## License

MIT

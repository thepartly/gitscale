# GitScale documentation

GitScale manages several git repositories as one workspace, from a single
`.gitscale.toml` file.

## Contents

**1. [Overview](overview.md)** — what GitScale is, and why it exists.

**2. Guides**

| | |
|---|---|
| 2.1 | [Declaring dependencies](dependencies.md) — adding and removing entries, artefacts, and pinning to a branch, tag or SHA |
| 2.2 | [Status](status.md) — reading `gitscale status`, every flag it prints, topics, and its JSON output |
| 2.3 | [Recursive dependencies](recursive-dependencies.md) — how every repository's requests resolve to one revision per major, overrides, implicit dependencies, and symlink dedup |
| 2.4 | [Everyday workflow](workflow.md) — `fetch`, `pull`, `push`, `sync`, `commit` |
| 2.5 | [Stores, worktrees and the CI cache](stores.md) — where checkouts come from, roots made of worktrees, moving and deleting, image pruning, and the `cache` commands |
| 2.6 | [Topics](topics.md) — one change across several repositories: `develop`, `upgrade`, `check`, and topics in CI |
| 2.7 | [Hooks](hooks.md) — `[hooks]` config commands, and installing GitScale as a git hook |
| 2.8 | [CI authentication](ci-authentication.md) — using the runner's own job token, with no pipeline setup |
| 2.9 | [Cleaning](clean.md) — `gitscale clean`, what it always keeps, and `--gc` |
| 2.10 | [Artefacts](artefacts.md) — publishing build output to an OCI registry, one image per commit, and installing it instead of a checkout or over one |

**3. [Configuration file reference](configuration.md)** — every table, key and
environment variable.

**4. [Command line reference](cli.md)** — every command, argument and flag.

**5. [Related tools](related-tools.md)** — how GitScale compares to submodules,
Google repo, west, vcstool and others.

---

[Contents](README.md) · [Next → 1. Overview](overview.md)

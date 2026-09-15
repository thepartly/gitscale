# GitScale documentation

GitScale manages several git repositories as one workspace, from a single
`.gitscale.toml` file.

## Contents

**1. [Overview](overview.md)** — what GitScale is, and why it exists.

**2. Guides**

| | |
|---|---|
| 2.1 | [Declaring dependencies](dependencies.md) — adding and removing entries, the three checkout modes, pinning to a branch, tag or SHA, and artefact storage |
| 2.2 | [Status](status.md) — reading `gitscale status`, every flag it prints, and its JSON output |
| 2.3 | [Recursive dependencies](recursive-dependencies.md) — transitive deps, hoisting to the root, symlink dedup, and the version-mismatch check |
| 2.4 | [Everyday workflow](workflow.md) — `clone`, `fetch`, `pull`, `push`, `sync`, `commit` |
| 2.5 | [The object cache](caching.md) — where objects come from, mirrors vs snapshots, what changes in CI, `dissociate`, `adopt_root`, and the `cache` commands |
| 2.6 | [Hooks](hooks.md) — `[hooks]` config commands, and installing GitScale as a git hook |
| 2.7 | [CI authentication](ci-authentication.md) — using the runner's own job token, with no pipeline setup |
| 2.8 | [Cleaning](clean.md) — `gitscale clean`, and what it always keeps |

**3. [Configuration file reference](configuration.md)** — every table, key and
environment variable.

**4. [Command line reference](cli.md)** — every command, argument and flag.

**5. [Related tools](related-tools.md)** — how GitScale compares to submodules,
Google repo, west, vcstool and others.

---

[Contents](README.md) · [Next → 1. Overview](overview.md)

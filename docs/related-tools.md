# 5. Related tools

- [Requirements](#the-requirements-we-have-to-cover-grouped)
- [Coverage](#coverage)
- [Design choices](#design-choices)
- [When to pick GitScale](#when-to-pick-gitscale)

GitScale occupies the same space as several multi-repo and vendoring tools. The comparison table includes a few rows of language-specific workspace managers for context rather than direct one-to-one equivalents.

## The requirements we have to cover, grouped

**R1 — Share code, configs and conventions of any kind**
* share typescript / rust / python clients and shared libraries
* share deployment configurations, like kustomize overlays, helm charts, and terraform modules (for the practice: deployment configs owned by teams / apps, but deployed by the central engine / release process)
* share development environments settings, like docker-compose and nix-shells
* share conventions and LLM skills

**R2 — Share large prebuilt payloads**
* share development datasets, large snapshots, etc.

**R3 — Partial / permission-scoped sharing**
* be able to import a dependency with less permissive access (eg. custom LLM provider code but no trained model data) to facilitate proactive offensive security approach, in other works be able to share repository code partially

**R4 — Version coherence across everything shared**
* have a simple way of identifying related versions of everything shared and corresponding git revisions / branches

**R5 — Direct and transitive dependency management**
* have a management for direct dependencies, which also exports / share different kinds of artefacts (eg. an app depends on repos exporting rust, python and conventions from 3 different repos)
* have a management for transitive dependencies, repo A depends on repo B, which depends on repo C, and so on and repos export / share different kinds of artefacts
* deduplicate shared transitive dependencies — if repo A and repo B both depend on repo C, C is checked out once, not copied per parent
* catch version mismatches instead of silently ending up with several revisions of the same dependency in one checkout

**R6 — Switch between SDK-only and full-stack imports**
* be able switch easily (or completely transparently) between importing SDKs only (eg. pure FE development without having local BE changed and deployed) or importing full stack (eg. full stack development with local BE changes and deployments)

**R7 — Fast, automatic, unsurprising DX**
* the process of importing a repository need to be fast and have good DX (eg. submodules copy entire codebase at imported revision)
* should work completely automatically (i.e not require --include-submodules or --recursive flags alike on checkout)
* should have great DX, default choices are super easy to understand and use, status is clear, automation is an addon but not a replacement for workflows people are used to

**R8 — Work with git worktrees, and reuse what is already on disk**
* a second worktree of the workspace should come up populated, without a manual per-dependency step after `git worktree add`
* checkout time should not scale with the number of worktrees — an existing local copy of a dependency should be reused instead of re-fetched
* the reuse must be safe to undo, since a borrowed copy that is later deleted or garbage-collected breaks the checkouts that borrowed from it

Baseline for every candidate: *should rely and work with git repositories.*

## Coverage

`✔` covered · `~` partial or with caveats · `✘` not covered

| Tool | Config | Approach | R1 any content | R2 large prebuilt | R3 partial share | R4 versions | R5 deps: transitive, dedup, mismatch | R6 SDK ↔ full stack | R7 auto + DX | R8 worktrees + local reuse |
|------|--------|----------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| **GitScale** | single `.gitscale.toml` | separate clones + symlinks | ✔ | ✔ S3 artefacts | ✔ artefact mode | ✔ one pinned config | ✔ recursive, symlink dedup, errors on conflicting revisions | ✔ readonly/artefact ↔ readwrite | ✔ git hooks, shallow, one status | ✔ auto-populated by hook, `--reference` reuse, opt-in `dissociate` |
| git submodules | `.gitmodules` + gitlink | separate clones | ✔ | ✘ | ✘ whole repo only | ✔ pinned SHA | `~` recursive, but nested copies and silent divergence | ✘ | ✘ needs `--recursive` | `~` `--reference` propagates, but `worktree add` leaves empty dirs |
| git subtree | none (in-tree) | merged into main tree | ✔ | ✘ | `~` per-prefix | `~` SHA buried in merges | ✘ | ✘ | `~` in-tree, awkward updates | ✔ in-tree — nothing extra to clone |
| [git-subrepo](https://github.com/ingydotnet/git-subrepo) | `.gitrepo` per subdir | merged into main tree | ✔ | ✘ | `~` per-subdir | ✔ `.gitrepo` records commit | ✘ | ✘ | `~` in-tree, extra binary | ✔ in-tree — nothing extra to clone |
| [Google repo](https://gerrit.googlesource.com/git-repo) | `manifest.xml` | separate clones | ✔ | ✘ | ✘ | ✔ manifest snapshot | ✘ flat manifest, no transitive | ✘ | ✘ manual `repo sync` | ✔ `--reference`, `--dissociate`, `--worktree` |
| [vcstool](https://github.com/dirk-thomas/vcstool) | `.repos` YAML | separate clones | ✔ | ✘ | ✘ | ✔ `.repos` pins | ✘ flat list, no transitive | ✘ | `~` manual `vcs import` | `~` `--shallow` only, no local reuse |
| [west](https://github.com/zephyrproject-rtos/west) | `west.yml` | separate clones | `~` mostly | ✘ | ✘ | ✔ `west.yml` | `~` manifest imports, name clashes error, no dedup | ✘ | `~` manual `west update` | `~` `update.auto-cache` reference cache, no worktree mode |
| [myrepos (mr)](https://myrepos.branchable.com/) | `.mrconfig` | separate clones, any VCS | ✔ | ✘ | ✘ | ✘ no pinning | ✘ | ✘ | `~` status only | ✘ |
| [meta](https://github.com/mateodelnorte/meta) | `.meta` JSON | separate clones + plugins | ✔ | ✘ | ✘ | ✘ no pinning | ✘ | ✘ | `~` plugin-dependent | ✘ |
| [gclient](https://chromium.googlesource.com/chromium/tools/depot_tools) | `DEPS` (Python) | separate clones + hooks | `~` mostly | `~` CIPD/GCS via hooks | ✘ | ✔ `DEPS` pins | `~` recursive DEPS, conflicts error, dedup is path-keyed | ✘ | ✘ manual sync, Python config | `~` `--cache-dir` shared clones, no worktree mode |
| Yarn workspaces | `package.json` | JS/TS monorepo workspace | ✘ JS/TS only | `~` registry tarballs | `~` published subset | ✔ lockfile | ✔ hoisting + range/peer checks, package-level | `~` `link` / resolutions | n/a single repo | n/a single repo |
| Cargo workspaces | `Cargo.toml` | Rust multi-crate workspace | ✘ Rust only | ✘ | `~` published crate | ✔ lockfile | ✔ semver unification, not repo dedup | `~` `[patch]` / path override | n/a single repo | n/a single repo |
| Go workspaces | `go.work` | Go multi-module workspace | ✘ Go only | ✘ | `~` published module | ✔ `go.mod` / `go.sum` | ✔ MVS picks one version, not repo dedup | `~` `go.work use` | n/a single repo | n/a single repo |

The three workspace managers work inside a single repo, so R7 and R8 never arise for them — they don't address multi-repo checkout at all.

## Design choices

- **Separate history per repo.** subtree and git-subrepo vendor code *into* your main repo, so the dependency's file history lives directly in the parent repository. That is what earns them R8 for free — a worktree of the parent already contains everything — but the cost is paid once per clone of the parent instead, whose history now carries every dependency's. GitScale keeps sub-repos as separate working trees by design, while the parent repo tracks the selected dependency revision in `.gitscale.toml`.
- **External binary.** Submodules and subtree ship with git and need no extra install; GitScale is a separate binary.
- **Git-only.** myrepos handles Git, Mercurial, Bazaar, SVN, and more. GitScale targets Git (plus its own artefact archives).
- **Simpler workflow model.** Google repo and west help coordinate branch creation, topic work, and release manifests across many repos. GitScale can pin each dependency to a branch, tag, or commit, but it does not try to manage a shared multi-repo branching lifecycle. When `.gitscale.toml` pins child repos to immutable tags or commit SHAs, that config effectively becomes the release manifest: the system version is defined as an assembly of specific subsystem versions.
- **R8 follows Google repo's design.** `repo init` has offered `--reference`, `--dissociate` and a `--worktree` mode for years, and gclient solves the same problem with a shared `--cache-dir` mirror. GitScale's approach is the same idea with the configuration removed: no mirror to set up and no flag to remember, because it works out the source workspace from git itself. The tradeoff is less control — repo lets you point at an arbitrary mirror, GitScale only reuses a workspace this one was actually derived from.
- **Self-contained tool.** meta has a plugin system and repo/gclient have larger surrounding ecosystems, while GitScale keeps the core workflow built into one tool. That simplifies operation and keeps behavior directly controllable for Partly's needs.
- **Linux-first symlink dedup.** Symlink-based dedup is efficient and natural on Linux, which is the primary development environment for Partly engineers. The tradeoff is that it is more awkward on Windows without developer mode or elevated privileges.

## When to pick GitScale

Choose GitScale when you want submodule-style separate checkouts but with a friendlier single config, enforced-readonly vendoring, prebuilt artefact delivery, and automatic deduplication of shared transitive dependencies. Prefer subtree/git-subrepo if you need everything in one repo and one history, or Google repo/west if you're managing a very large manifest-driven project with heavy multi-branch workflows.

---

[← 4. Command line reference](cli.md) · [Contents](README.md)

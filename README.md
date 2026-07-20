# GitScale

Manage multiple sub-repositories from a single config file. An alternative to git submodules — simpler, with readonly enforcement, artefact mode, and cloud storage for pre-built archives.

## Install

```
pip install gitscale
```

Requires Python 3.12+.

## Quick start

Create a `.gitscale.toml` in your project root:

```toml
[repos]
"libs/core" = { url = "https://github.com/org/core.git", revision = "main", mode = "readonly" }
"libs/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
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
      REPO          PATH   MODE        REF    EXPECTED   STATUS
✔     libs/core     -      readonly    main   main       ok
✔     libs/utils    -      readwrite   v2.1   v2.1       ok
⤷     libs/shared   ../s   readonly    main   main       symlink
```

## Config file

The `.gitscale.toml` file lives at the root of your project. GitScale searches upward from the current directory to find it.

### Repos

```toml
[repos]
"libs/core" = { url = "git@github.com:org/core.git", revision = "main", mode = "readonly" }
"libs/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
"meta/frontend" = { url = "https://github.com/org/frontend.git", revision = "main", mode = "artefact" }
```

Each entry maps a local directory to a git repo:

- **url** (required) — Git repository URL (HTTPS or SSH)
- **revision** — Branch, tag, or commit SHA. Defaults to the repo's default branch
- **mode** — Access mode:
  - `readwrite` (default) — Normal clone, full access
  - `readonly` — Cloned, but all files have write permissions removed
  - `artefact` — No git clone. Artefact archive synced via cloud storage
- **recursive** — Scan the repo for a nested `.gitscale.toml` and resolve transitive deps (default: `true`)

### Storage

Configure cloud storage for artefact data:

```toml
[storage]
url = "https://my-bucket.s3.us-east-1.amazonaws.com/gitscale"
```

See [Artefact storage](#artefact-storage) below.

## Commands

### `gitscale clone [NAMES...]`

Clone sub-repositories that don't exist locally yet. Artefact entries are downloaded from cloud storage.

```
gitscale clone                  # clone all
gitscale clone libs/core        # clone one
```

### `gitscale fetch [NAMES...]`

Fetch latest remote state without modifying local files. For git repos: `git fetch`. For artefacts: HEAD request to check remote ETag.

```
gitscale fetch                  # fetch all
gitscale fetch libs/core        # fetch one
```

### `gitscale pull [NAMES...]`

Pull latest changes. For git repos: `git pull --ff-only`. For artefacts: downloads from cloud storage if remote is newer. Clones repos that don't exist yet.

```
gitscale pull                   # pull all
gitscale pull libs/core         # pull one
```

### `gitscale push [NAMES...]`

Push local changes. For git repos: `git push`. Readonly and artefact entries are skipped.

```
gitscale push                   # push all
gitscale push libs/core         # push one
```

### `gitscale sync [NAMES...]`

Full sync: clone + pull + push in sequence.

```
gitscale sync                   # sync all
gitscale sync libs/core         # sync one
```

### `gitscale status`

Show the status of all managed repos. Use `--fetch` to check remote state before reporting.

```
gitscale status                 # table output
gitscale status --format json   # JSON output
gitscale status --fetch         # fetch before checking
```

Columns:

| Column | Description |
|--------|-------------|
| REPO | Directory name of the managed repo |
| PATH | Symlink target path (or `-` if not a symlink) |
| MODE | `readonly`, `readwrite`, or `artefact` |
| REF | Current git ref (branch/tag/SHA) |
| EXPECTED | Declared revision from `.gitscale.toml` |
| STATUS | Flags describing repo state |

Status icons and flags:

| Icon | Color | Flag | Meaning |
|------|-------|------|---------|
| `✔` | green | **ok** | Clean, on expected ref |
| `⤷` | cyan | **symlink** | Resolved as symlink to parent-level checkout |
| `~` | yellow | **unlinked** | Expected symlink replaced by real clone (safe to relink) |
| `~` | red | **unlinked, modified** | Unlinked clone has local changes (unsafe to relink) |
| `~` | red | **unlinked, dirty** | Unlinked clone is clean, but the parent repo has uncommitted changes  (unsafe to relink) |
| `!` | red | **dirty** | Uncommitted changes |
| `≠` | red | **ref-mismatch** | On a different branch than declared |
| `≠` | red | **stale** | Shallow clone: local differs from upstream |
| `⇑` | yellow | **+N** | Commits ahead of upstream |
| `⇓` | yellow | **-N** | Commits behind upstream |
| `⇅` | yellow | **+N, -N** | Diverged (ahead and behind) |
| `◆` | cyan | **detached** | HEAD is detached |
| `✘` | red | **missed** | Directory doesn't exist yet |

### `gitscale add DIRECTORY REPO_URL REVISION`

Add a new entry to `.gitscale.toml`.

```
gitscale add libs/core https://github.com/org/core.git main
gitscale add libs/core https://github.com/org/core.git main --mode readonly
gitscale add meta/svc https://github.com/org/svc.git main --mode artefact
```

### `gitscale remove DIRECTORY`

Remove an entry from `.gitscale.toml`.

```
gitscale remove libs/core
```

### Global options

- `-v, --verbose` — Enable verbose output (must go before the subcommand)
- `--version` — Show version
- `-C, --root PATH` — Override the root directory (available on all subcommands)

```
gitscale -v sync
gitscale --version
gitscale -C /path/to/project status
```

## Repo modes

### readwrite (default)

Normal git clone. Full read/write access to files.

### readonly

Shallow-cloned (`--depth 1`) locally, then all file write permissions are stripped (excluding `.git/`). Git operations (sync, pull) use `fetch --depth 1` + `reset --hard` to update.

Useful for vendored dependencies you shouldn't modify.

### artefact

No git clone. An artefact archive (`<revision>.tar.gz`) is downloaded from cloud storage and extracted into the entry's directory on `clone` and `pull`. All extracted files are kept readonly. ETag-based change detection avoids unnecessary transfers. Push is not supported (artefacts are published by CI).

The archive content is opaque to gitscale — it can contain anything.

## Shallow clones

Readonly repos are always shallow-cloned (`--depth 1 --branch <revision>`). This is fast and disk-efficient — only the declared revision is fetched.

When the `CI` environment variable is set to `1` or `true` (as done by GitHub Actions, GitLab CI, etc.), **all** git repos are shallow-cloned, regardless of mode.

| Context | readwrite | readonly | artefact |
|---------|-----------|----------|----------|
| Local | full clone | shallow | no git |
| CI (`CI=1`) | shallow | shallow | no git |

Shallow repos show `≠ stale` in status when the local commit differs from upstream (exact behind count is unavailable).

## Artefact storage

### Setup

Add a `[storage]` section to your config:

```toml
[storage]
url = "https://my-bucket.s3.us-east-1.amazonaws.com/gitscale"
```

### Supported backends

Any S3-compatible storage works with the same URL pattern:

| Service | Example URL |
|---------|------------|
| AWS S3 | `https://my-bucket.s3.us-east-1.amazonaws.com/prefix` |
| MinIO | `https://minio.corp.com:9000/my-bucket/prefix` |
| Cloudflare R2 | `https://<account>.r2.cloudflarestorage.com/my-bucket/prefix` |
| Backblaze B2 | `https://s3.us-west-004.backblazeb2.com/my-bucket/prefix` |
| DigitalOcean Spaces | `https://nyc3.digitaloceanspaces.com/my-bucket/prefix` |
| Google Cloud Storage | `https://storage.googleapis.com/my-bucket/prefix` |
| Local directory | `/tmp/artefacts` or `file:///home/user/artefacts` |

### Authentication

Set credentials via environment variables (not needed for local storage):

**S3-compatible** (AWS, MinIO, R2, B2, Spaces):
```
export AWS_ACCESS_KEY_ID=your-key
export AWS_SECRET_ACCESS_KEY=your-secret
export AWS_DEFAULT_REGION=us-east-1   # optional, detected from URL
```

**Google Cloud Storage:**
```
export GOOGLE_TOKEN=$(gcloud auth print-access-token)
```

### Object layout

Artefact archives are stored at: `{storage_url}/{host}/{owner}/{repo}/{revision}.tar.gz`

For example, with `url = "https://bucket.s3.amazonaws.com/meta"` and a repo at `https://github.com/org/app.git` on revision `main`:

```
https://bucket.s3.amazonaws.com/meta/github.com/org/app/main.tar.gz
```

## Hooks

Add a `[hooks]` section to run commands after certain operations:

```toml
[hooks]
post_sync = "make install"
```

- **post_sync** — Runs after `pull` and `sync` complete (executed via `sh -c` in the config root directory). Fails the command if the hook exits non-zero.

## Recursive dependencies

When a cloned repo contains its own `.gitscale.toml`, GitScale resolves transitive dependencies automatically. Instead of cloning nested copies, it creates **symlinks** from the child's declared paths to the root-level checkouts.

### Example

```
# Root .gitscale.toml
[repos]
"core" = { url = "git@github.com:org/core.git", revision = "main" }
"sharedlibs" = { url = "git@github.com:org/sharedlibs.git", revision = "main" }
```

If `core/.gitscale.toml` declares:

```toml
[repos]
"libs/shared" = { url = "git@github.com:org/sharedlibs.git", revision = "main" }
```

GitScale matches the URL, skips cloning, and creates a symlink: `core/libs/shared → ../../sharedlibs`.

### Rules

- **Recursive by default.** Disable per repo with `recursive = false`.
- **Root is the source of truth.** All transitive deps must be declared in the root config — if missing, GitScale errors out.
- **Root revision wins.** If root specifies a revision, child declarations are ignored silently.
- **Revision deferral.** If root omits `revision`, the child's revision is adopted. If multiple children disagree, GitScale errors out and asks you to pin one in the root config.
- **Works with artefacts.** Extracted artefact folders are scanned for `.gitscale.toml` too.

## Related tools

GitScale occupies the same space as several multi-repo and vendoring tools. The last few rows include adjacent language-specific workspace managers for context rather than direct one-to-one equivalents.

| Tool | Config | Approach | History in parent repo | Prebuilt artefacts | Dedup transitive deps | Language agnostic? |
|------|--------|----------|------------------------|--------------------|-----------------------|--------------------|
| **GitScale** | single `.gitscale.toml` | separate clones + symlinks | config-pinned revision | yes (S3) | yes (symlinks) | yes |
| git submodules | `.gitmodules` + gitlink | separate clones | pinned SHA only | no | no (nested copies) | yes |
| git subtree | none (in-tree) | merged into main tree | yes (full) | no | no | yes |
| [git-subrepo](https://github.com/ingydotnet/git-subrepo) | `.gitrepo` per subdir | merged into main tree | yes (squashed) | no | no | yes |
| [Google repo](https://gerrit.googlesource.com/git-repo) | `manifest.xml` | separate clones | no | no | no | yes |
| [vcstool](https://github.com/dirk-thomas/vcstool) | `.repos` YAML | separate clones | no | no | no | yes |
| [west](https://github.com/zephyrproject-rtos/west) | `west.yml` | separate clones | no | no | no | mostly |
| [myrepos (mr)](https://myrepos.branchable.com/) | `.mrconfig` | separate clones, any VCS | no | no | no | yes |
| [meta](https://github.com/mateodelnorte/meta) | `.meta` JSON | separate clones + plugins | no | no | no | yes |
| [gclient](https://chromium.googlesource.com/chromium/tools/depot_tools) | `DEPS` (Python) | separate clones + hooks | no | no | no | mostly |
| Yarn workspaces | `package.json` | JS/TS monorepo workspace | yes (full) | no | package-level hoisting | no |
| Cargo workspaces | `Cargo.toml` | Rust multi-crate workspace | yes (full) | no | shared workspace, not repo dedup | no |
| Go workspaces | `go.work` | Go multi-module workspace | yes (full) | no | module-level, not repo dedup | no |

### GitScale highlights

- **Readonly enforcement.** Vendored dependencies have their write bits stripped on disk, so accidental edits fail loudly instead of drifting silently. Submodules, repo, and vcstool leave everything writable.
- **Artefact mode.** Entries can be pulled as prebuilt `tar.gz` archives from any S3-compatible bucket instead of cloned — useful for large generated outputs or closed-source blobs. No comparable tool ships this; you'd otherwise bolt on a separate artefact/LFS pipeline.
- **Native S3 storage.** Signing and transfer are built in (no `aws` CLI or SDK required); works with AWS, MinIO, R2, B2, Spaces, GCS, or a local directory.
- **Transitive dedup via symlinks.** When two nested configs depend on the same repo, GitScale checks it out once at the root and symlinks the rest, avoiding duplicate clones. Submodules and repo produce independent nested copies.
- **CI-aware shallow cloning.** Automatically shallow-clones everything under `CI=1`, and readonly repos are always shallow — faster, smaller checkouts without extra flags.
- **Rich, single-glance status.** One `status` table (with JSON output) surfaces ahead/behind, detached, ref-mismatch, dirty, stale, and broken-symlink states across every repo.
- **One human-readable config.** A single TOML file versus `.gitmodules` + gitlink entries, XML manifests, or Python `DEPS`.

### Design choices

- **Separate history per repo.** subtree and git-subrepo vendor code *into* your main repo, so the dependency's file history lives directly in the parent repository. GitScale keeps sub-repos as separate working trees by design, while the parent repo tracks the selected dependency revision in `.gitscale.toml`.
- **External binary.** Submodules and subtree ship with git and need no extra install; GitScale is a separate binary.
- **Git-only.** myrepos handles Git, Mercurial, Bazaar, SVN, and more. GitScale targets Git (plus its own artefact archives).
- **Simpler workflow model.** Google repo and west help coordinate branch creation, topic work, and release manifests across many repos. GitScale can pin each dependency to a branch, tag, or commit, but it does not try to manage a shared multi-repo branching lifecycle. When `.gitscale.toml` pins child repos to immutable tags or commit SHAs, that config effectively becomes the release manifest: the system version is defined as an assembly of specific subsystem versions.
- **Self-contained tool.** meta has a plugin system and repo/gclient have larger surrounding ecosystems, while GitScale keeps the core workflow built into one tool. That simplifies operation and keeps behavior directly controllable for Partly's needs.
- **Linux-first symlink dedup.** Symlink-based dedup is efficient and natural on Linux, which is the primary development environment for Partly engineers. The tradeoff is that it is more awkward on Windows without developer mode or elevated privileges.

### When to pick GitScale

Choose GitScale when you want submodule-style separate checkouts but with a friendlier single config, enforced-readonly vendoring, prebuilt artefact delivery, and automatic deduplication of shared transitive dependencies. Prefer subtree/git-subrepo if you need everything in one repo and one history, or Google repo/west if you're managing a very large manifest-driven project with heavy multi-branch workflows.

## License

MIT

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
    REPO          MODE        REF    EXPECTED   STATUS
✔   .                         main              ok
✔   libs/core     readonly    main   main       ok
✔   libs/utils    readwrite   v2.1   v2.1.0     ok
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

Show the status of all managed repos, including the root repo itself (shown as `.`). Use `--fetch` to check remote state before reporting.

```
gitscale status                 # table output
gitscale status --format json   # JSON output
gitscale status --fetch         # fetch before checking
```

Status icons and flags:

| Icon | Color | Flag | Meaning |
|------|-------|------|---------|
| `✔` | green | **ok** | Clean, on expected ref |
| `!` | orange | **dirty** | Uncommitted changes |
| `≠` | orange | **ref-mismatch** | On a different branch than declared |
| `≠` | orange | **stale** | Shallow clone: local differs from upstream |
| `⇑` | yellow | **+N** | Commits ahead of upstream |
| `⇓` | yellow | **-N** | Commits behind upstream / artefact remote is newer |
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

## License

MIT

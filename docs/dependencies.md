# 2.1 Declaring dependencies

- [The config file](#the-config-file)
- [Ignoring the checkout directory](#ignoring-the-checkout-directory)
- [Adding and removing entries](#adding-and-removing-entries)
- [Checkout modes](#checkout-modes)
  - [readwrite](#readwrite-default)
  - [readonly](#readonly)
  - [artefact](#artefact)
- [Pinning a revision](#pinning-a-revision)
  - [Branch](#branch)
  - [Tag](#tag)
  - [Commit SHA](#commit-sha)
  - [No revision](#no-revision)
- [Shallow clones](#shallow-clones)
- [Recursive dependencies](#recursive-dependencies)
- [Artefact storage](#artefact-storage)
  - [Supported backends](#supported-backends)
  - [Credentials](#credentials)
  - [Object layout](#object-layout)
  - [Publishing artefacts](#publishing-artefacts)

## The config file

`.gitscale.toml` lives at the root of the workspace. Every command searches
upward from the current directory until it finds one, so commands work from
anywhere inside the workspace. `-C, --root PATH` overrides where the search
starts.

```toml
[repos]
"imports/core"  = { url = "git@github.com:org/core.git", revision = "main", mode = "readonly" }
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
"meta/frontend" = { url = "https://github.com/org/frontend.git", revision = "main", mode = "artefact" }
"vendor/tools"  = { url = "https://github.com/org/tools.git", revision = "main", recursive = false }
```

The key is the directory the checkout lands in, relative to the config. Each
entry takes:

| Key | Required | Default | Meaning |
|---|---|---|---|
| `url` | yes | — | Git repository URL: HTTPS, SSH (`git@host:owner/repo.git` or `ssh://…`), or a local path |
| `revision` | no | the remote's default branch | Branch, tag or commit SHA — see [pinning a revision](#pinning-a-revision) |
| `mode` | no | `readwrite` | `readwrite`, `readonly` or `artefact` — see [checkout modes](#checkout-modes) |
| `recursive` | no | `true` | Whether to read this repo's own `.gitscale.toml` — see [recursive dependencies](recursive-dependencies.md) |

Directories must be relative and free of `..`; URLs and revisions may not start
with `-` or use a `helper::` remote-helper prefix. The reasoning, and the full
list of values GitScale refuses, is in
[configuration → values refused](configuration.md#values-gitscale-refuses-to-pass-to-git).

## Ignoring the checkout directory

Keep every declared checkout under one directory — `imports/` throughout this
documentation — and **add it to the workspace repository's `.gitignore`**:

```gitignore
/imports/
```

The checkouts are not content of the workspace repository. What that repository
tracks is the *selection*: `.gitscale.toml`, which names each repository and the
revision it is pinned to. The checkouts themselves are reproduced from it by
`gitscale clone`.

Without the ignore rule, every checkout is an untracked directory in the
workspace repository, which has three visible consequences:

- `git status` in the workspace repository is permanently noisy, listing
  directories GitScale put there. The same happens one level down, where it also
  shows up in [`gitscale status`](status.md): a sub-repository that declares its
  own dependencies reads as [`dirty`](status.md#status-flags) purely because of
  the symlinks planted in it.
- [`gitscale commit`](workflow.md#commit) with no names runs `git add -A` in the
  workspace repository. A git checkout would be staged as an embedded
  repository; an [artefact](#artefact) checkout, which has no `.git` directory
  at all, would have its entire extracted contents committed.
- Anyone reading a diff has to scroll past it.

The same applies one level down. A repository that declares its own dependencies
gets [symlinks](recursive-dependencies.md) planted at those paths in its working
tree, and those are untracked files in *that* repository — so it should ignore
its own import directory in its own `.gitignore`.

Ignoring is safe with [`gitscale clean`](clean.md), which removes ignored files
(`git clean -xd`) but always excludes the declared checkouts and GitScale's own
symlinks, at every level. It is **not** safe with a hand-run `git clean -xdf` at
the workspace root, which has no such knowledge and will delete the whole
workspace.

Nothing enforces the `imports/` name or requires the checkouts to share a
directory — an entry may be declared at any relative path. One ignored directory
is simply the arrangement that keeps the workspace repository clean with a
single line.

## Adding and removing entries

```
gitscale add imports/core https://github.com/org/core.git main
gitscale add imports/core https://github.com/org/core.git main --mode readonly
gitscale add meta/svc  https://github.com/org/svc.git  main --mode artefact

gitscale remove imports/core
```

`add` takes `DIRECTORY REPO_URL REVISION`, all three required, plus an optional
`--mode`. It refuses a directory that is already declared. If no config exists
yet, it creates one next to `--root` (or the current directory).

Neither command touches the filesystem — they edit `.gitscale.toml` only. Run
[`gitscale clone`](workflow.md#clone) afterwards to materialise a new entry;
remove the directory by hand after `gitscale remove`, since
[`clean`](clean.md) deliberately does not delete checkouts.

> **Both commands rewrite the whole file.** The config is re-emitted from what
> GitScale parsed, so comments, key order and formatting are lost, and any table
> GitScale does not know about is dropped. Edit `.gitscale.toml` by hand if you
> keep comments in it. Entries always come back with `recursive = true` unless
> the file already said otherwise.

## Checkout modes

| | Clone | Files on disk | `push` | Objects |
|---|---|---|---|---|
| `readwrite` | full git clone | writable | pushed | git |
| `readonly` | git clone, write bits stripped | read-only | skipped | git |
| `artefact` | no clone at all | read-only | skipped | `tar.gz` from object storage |

### readwrite (default)

An ordinary git clone with full read/write access. This is the mode for a
repository you are actually developing in: `pull` fast-forwards it, `push`
pushes it, `commit` commits it.

### readonly

Cloned like any other repository, then every file in the working tree has its
write bits cleared (`.git/` is left alone, and symlinks are skipped). Use it for
vendored dependencies nobody should edit in place: an accidental write fails
immediately instead of producing a change that later gets lost.

`pull` and `sync` restore write permission, update the checkout, then strip it
again. `push` and `commit` skip readonly entries.

Readonly repos are shallow-cloned when GitScale is talking to a remote directly;
with the [object cache](caching.md) on — the default — they are cloned in full
from a local mirror instead, because borrowing from a mirror makes depth
pointless. See [shallow clones](#shallow-clones).

### artefact

No git clone happens. GitScale downloads `<revision>.tar.gz` from the configured
[artefact storage](#artefact-storage) and extracts it into the entry's
directory. The archive's content is opaque to GitScale — it can hold anything.

- `clone` downloads and extracts.
- `fetch` issues a HEAD request and records the remote ETag in `.etag-remote`.
- `pull` compares ETags, and re-downloads only when the remote one differs. On
  re-download the directory's contents are removed and replaced (files whose
  name starts with `.`, such as the ETag markers, are kept).
- `push` and `commit` skip artefact entries: artefacts are published by whatever
  builds them, not by GitScale.

Two marker files live in the directory: `.etag` (what is extracted right now)
and `.etag-remote` (what the last HEAD saw). `gitscale status` reports an
artefact as behind when they differ.

Top-level files in the extracted directory have their write bits cleared. Files
inside subdirectories and dot-files are left as the archive made them.

Artefact entries are never put in the object cache — an unpacked archive has no
object store — and they are skipped by [`clean`](clean.md).

## Pinning a revision

`revision` accepts three kinds of value, and what ends up checked out differs.

### Branch

```toml
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "main" }
```

The clone lands **on that branch**, tracking `origin/main`. `pull` fast-forwards
it (`--ff-only`); a branch that has diverged is left alone rather than forced.

### Tag

```toml
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "v2.1.0" }
```

Git leaves **HEAD detached** at the commit the tag points at — that is what git
itself does for a tag, and GitScale does not fight it. `gitscale status` knows
this, and does not report a mismatch as long as HEAD is at the right commit.

### Commit SHA

```toml
"imports/utils" = { url = "https://github.com/org/utils.git", revision = "9fceb02..." }
```

`git clone --branch` accepts only branch and tag names, so a SHA cannot be
cloned shallowly the usual way. When a shallow clone is called for, GitScale
builds the repository directly instead:

```sh
git init && git remote add origin <url>
git fetch --depth 1 origin <sha>
git checkout --detach FETCH_HEAD
```

The result is a shallow clone with a detached HEAD at exactly the pinned commit.
This matters most in CI, where everything is shallow — an entry that clones fine
locally would otherwise fail there.

Some git servers refuse to serve an arbitrary commit. If the fetch is rejected,
GitScale falls back to a full clone and checks the revision out normally. GitHub
and GitLab both permit it.

> **A revision of 7–64 hexadecimal characters is treated as a SHA.** A branch or
> tag whose name happens to be entirely hexadecimal (`abcdef1`) therefore takes
> the SHA path too. It still resolves to the right commit, but the checkout ends
> up detached rather than on the branch. Rename the ref if you need to stay on
> it.

### No revision

Omitting `revision` leaves the choice to the remote's default branch — except
where a [recursive dependency](recursive-dependencies.md) supplies one, which is
how a root config can defer the decision to the repositories that actually care.

For an artefact entry, an omitted revision means the archive is looked up under
the name `HEAD`.

## Shallow clones

Depth depends on where the objects come from, not only on the mode:

| Context | readwrite | readonly | artefact |
|---|---|---|---|
| Local, cache on (default) | full clone | full clone | no git |
| Local, `--no-cache` | full clone | shallow | no git |
| CI (`CI=1` or `CI=true`) | shallow | shallow | no git |

With the [object cache](caching.md) on, a developer machine borrows from a full
mirror, where shallow buys nothing and borrows less cleanly — so a readonly repo
is cloned whole and its history is there to browse at no extra cost. CI is
served by shallow snapshot entries and keeps the depth-1 checkout it has today.

A shallow repo cannot report an exact behind count, so `gitscale status` shows
`≠ stale` when its commit differs from upstream.

## Recursive dependencies

If a checked-out repository carries its own `.gitscale.toml`, GitScale reads it,
requires everything it declares to also be declared at the root, and creates a
symlink instead of a second checkout. Turn it off per entry with
`recursive = false`. The whole mechanism — hoisting, dedup, and the
version-mismatch check — is [its own page](recursive-dependencies.md).

## Artefact storage

`mode = "artefact"` entries need a `[storage]` table saying where archives live:

```toml
[storage]
url = "https://my-bucket.s3.us-east-1.amazonaws.com/gitscale"
```

If the table is present, `url` is required and must be non-empty. Without a
`[storage]` url, artefact entries fail with `no [storage] configured`; git
entries are unaffected.

### Supported backends

Any S3-compatible service works with the same URL shape. Requests are signed
with AWS Signature V4 built into GitScale — no `aws` CLI or SDK involved.

| Service | Example URL |
|---|---|
| AWS S3 | `https://my-bucket.s3.us-east-1.amazonaws.com/prefix` |
| MinIO | `https://minio.corp.com:9000/my-bucket/prefix` |
| Cloudflare R2 | `https://<account>.r2.cloudflarestorage.com/my-bucket/prefix` |
| Backblaze B2 | `https://s3.us-west-004.backblazeb2.com/my-bucket/prefix` |
| DigitalOcean Spaces | `https://nyc3.digitaloceanspaces.com/my-bucket/prefix` |
| Google Cloud Storage | `https://storage.googleapis.com/my-bucket/prefix` |
| Local directory | `/srv/artefacts` or `file:///srv/artefacts` |

A URL on `storage.googleapis.com` is detected as GCS and uses bearer-token auth;
one starting with `/`, `./` or `file://` is read straight off the filesystem;
everything else is treated as S3.

### Credentials

Set through the environment. Local-directory storage needs none.

S3-compatible (AWS, MinIO, R2, B2, Spaces):

```
export AWS_ACCESS_KEY_ID=your-key
export AWS_SECRET_ACCESS_KEY=your-secret
export AWS_DEFAULT_REGION=us-east-1   # optional; otherwise derived from the URL, else us-east-1
```

Google Cloud Storage:

```
export GOOGLE_TOKEN=$(gcloud auth print-access-token)
# GCLOUD_ACCESS_TOKEN is accepted as well
```

### Object layout

```
{storage_url}/{host}/{owner}/{repo}/{revision}.tar.gz
```

Slashes in a revision become `_`, so `release/1.2` is stored as `release_1.2`.
An empty revision is looked up as `HEAD` by the commands.

With `url = "https://bucket.s3.amazonaws.com/meta"` and an entry at
`https://github.com/org/app.git` on revision `main`:

```
https://bucket.s3.amazonaws.com/meta/github.com/org/app/main.tar.gz
```

### Publishing artefacts

GitScale consumes artefacts; it has no publish command. Whatever builds the
archive — normally a CI job in the source repository — writes it to that path
itself. Extraction refuses any archive entry whose path escapes the destination
directory.

---

[← 1. Overview](overview.md) · [Contents](README.md) · [Next → 2.2 Status](status.md)

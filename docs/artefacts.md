# 2.9 Artefacts

- [How it works](#how-it-works)
- [Declaring an artefact entry](#declaring-an-artefact-entry)
- [Which commit an entry gets](#which-commit-an-entry-gets)
- [Publishing](#publishing)
  - [What gets published](#what-gets-published)
  - [Patterns](#patterns)
  - [Groups and layers](#groups-and-layers)
  - [artefact publish](#artefact-publish)
- [Looking at what is published](#looking-at-what-is-published)
  - [artefact show](#artefact-show)
  - [artefact list](#artefact-list)
- [Where artefacts live](#where-artefacts-live)
- [Logging in](#logging-in)
- [Pipelines](#pipelines)
  - [One-time setup](#one-time-setup)
  - [GitLab](#gitlab)
  - [GitHub](#github)
- [The checkout](#the-checkout)
- [Status](#status)
- [Caching](#caching)

`mode = "artefact"` puts a repository's **build output** in the workspace
instead of a git checkout: the files its own pipeline produced, read-only. The
source repository publishes them to an OCI registry — GitLab's, GHCR, or any
other — as one image per commit, and every workspace that declares the entry
installs the image of the commit its revision names.

## How it works

![Consumers pull the image tagged with the commit their revision resolves to](images/artefacts-flow.svg)

Git decides which commit a revision means; the registry only stores one image
per commit. The tag of every image is the **full commit SHA** it was built
from, so an artefact always matches what a git checkout of the same entry
would get — and a commit with no image is an error, never an older image
passed off as current.

## Declaring an artefact entry

Nothing beyond what a git entry has. The image's location follows from `url`:

```toml
[repos]
"meta/frontend" = { url = "git@gitlab.com:org/frontend.git", revision = "main", mode = "artefact" }
"meta/tools"    = { url = "https://github.com/org/tools.git", revision = "v2.1.0", mode = "artefact" }
"meta/app"      = { url = "https://github.com/org/app.git", mode = "artefact" }   # default branch
"meta/pinned"   = { url = "https://github.com/org/lib.git", revision = "9fceb02d0ae598e95dc970b74767f19372d61af8", mode = "artefact" }
```

## Which commit an entry gets

Every revision is resolved to a commit on the remote first, with one `git
ls-remote` — through the same [CI credentials](ci-authentication.md) git
entries use — and that commit's image is what gets installed.

| `revision` | Resolved with | |
|---|---|---|
| Branch | `ls-remote <url> refs/heads/<name>` | Matched exactly: `main` never finds `feature/main` |
| Tag | `ls-remote <url> refs/tags/<name>` | An annotated tag resolves to the commit it points at |
| Full SHA (40 or 64 hex) | used as given | No network until the download |
| Empty | `ls-remote --symref <url> HEAD` | The remote's default branch, whatever it is called today |
| Abbreviated SHA | **refused** | Taken for a branch or tag name, which the remote does not have; the error says to run `git rev-parse <short>` in a checkout of the source |

A commit is only ever a full SHA, as [for every entry](dependencies.md#commit-sha);
a branch or tag whose name happens to be all hex digits (`20241001`) works as
the ref it is.

When the commit has no image the command fails —
[`artefact show`](#artefact-show) says what the remote and the registry hold:

```
FAIL  meta/app: no artefact for ghcr.io/org/app/gitscale:3f2a9c1 (main); its pipeline may not have published yet
```

## Publishing

### What gets published

The source repository's own `.gitscale.toml` says which files to ship, as
groups of glob patterns. Each group becomes one layer of the image; nobody
builds or handles an archive by hand.

```toml
[artefact]
root = "dist"            # optional; patterns and archive paths are relative to it

[[artefact.layer]]
name    = "vendor"
include = ["vendor/**"]

[[artefact.layer]]
name    = "app"
include = ["**/*.js", "**/*.css", "index.html"]
exclude = ["**/*.map"]
```

With one group, the layer tables can be left out:

```toml
[artefact]
root    = "dist"
include = ["**"]
exclude = ["**/*.map"]
```

`root` defaults to the directory the config is in. Every key of `[artefact]` is
checked when the config is read: an unknown key, a pattern that leaves `root`,
or a group with nothing to include is an error, not a quietly smaller artefact.

### Patterns

Plain globs with separate `include` and `exclude` lists — the convention of
GitLab's `artifacts:paths` and GitHub's `upload-artifact`. Not gitignore
syntax, whose no-slash, anchoring and `!` rules make it easy to ship more or
less than meant.

| Pattern | Matches |
|---|---|
| `**` | every file under `root` |
| `vendor/**` | everything under `vendor/`, at any depth |
| `vendor` | the same: naming a directory includes its contents |
| `*.html` | `.html` files directly in `root` only |
| `**/*.js` | `.js` files at any depth |
| `assets/*.png` | `.png` files directly in `assets/`, not in subdirectories |
| `{img,fonts}/**` | everything under `img/` or `fonts/` |
| `report-?.pdf` | `?` is exactly one character |

- `*` stops at `/`; `**` crosses it.
- Patterns are relative to `root`, and may not start with `/` or contain `.` or
  `..` components.
- Files are found by walking the filesystem, not git: build output is usually
  ignored. `.git` is never shipped.

### Groups and layers

- **One layer per group, in the order listed.** The group's name is the
  layer's `org.opencontainers.image.title`.
- **First match wins.** A file matching several groups goes to the earliest,
  so a catch-all last group (`include = ["**"]`) is safe, and no file is in two
  layers.
- **Unmatched files are not shipped.**
- **A group that matches nothing fails the publish**, so a broken build or a
  typo cannot publish an artefact with a hole in it.
- **Layers are reproducible gzip tars**: entries sorted, ownership and times
  fixed, modes reduced to 0644 or 0755, a gzip header with no time or name. The
  same files give the same digests, build after build.
- **Executable bits are kept.** Symlinks are kept when they point inside
  `root`; one that points outside fails the publish.
- **Consumers download only the layers they do not already hold**: when only
  `app` changes, `vendor` is not fetched again.

Split along how often things change: a large dependency layer that changes
once a month, and a small application layer that changes every commit.

**The repository's own `.gitscale.toml` is always shipped**, as a first layer
of its own named `gitscale`, at the top of the artefact — even when `root` is a
subdirectory that does not contain it. It is how a consumer learns the
artefact's [dependencies](recursive-dependencies.md): resolution downloads just
that layer, a few hundred bytes, before deciding anything else, and keeps it in
the cache, where every commit that left the file alone shares it. A
`.gitscale.toml` at the top of `root` is not shipped, since it would clash with
it; and a group may not be named `gitscale`. The dependencies are linked inside
the extracted artefact like any checkout's.

### artefact publish

```
gitscale artefact publish [-C DIR] [--commit SHA] [--force] [--dry-run]
```

Run in the source repository's pipeline, after the build:

1. **Pack the groups** as described above.
2. **Identify the source.** The commit is `--commit`, else the job's own
   (`CI_COMMIT_SHA` in a GitLab job, `GITHUB_SHA` in GitHub Actions), else
   `git rev-parse HEAD`. The repository is the job's project (`CI_PROJECT_URL`;
   `GITHUB_SERVER_URL` + `GITHUB_REPOSITORY`), else the `origin` remote with
   any credentials removed. The image name comes from the same mapping
   consumers use, so the two always agree.
3. **Check the tag.** If the commit's tag already holds the same layers, it
   says `Already published` and stops: a retried job is a no-op. If it holds
   different layers, it fails — two builds of one commit should produce the
   same files — unless `--force` is given.
4. **Push** the layers the registry does not already have, a config blob, then
   the manifest under the commit's tag. The manifest carries
   `org.opencontainers.image.revision` (the commit) and
   `org.opencontainers.image.source` (the repository URL, which GHCR uses to
   link the package to the repository), and no creation time, so its digest is
   reproducible too.

```
Publishing ghcr.io/org/app/gitscale:9fceb02d0ae598e95dc970b74767f19372d61af8
  layer gitscale: 1 file, 312 B, sha256:e91d…
  layer vendor: 412 files, 3.1 MiB, sha256:4c1f…
  layer app: 12 files, 84.0 KiB, sha256:a90e…
  gitscale already in the registry
  vendor already in the registry
  pushed app
  pushed config
Published ghcr.io/org/app/gitscale@sha256:77d0…
```

`--dry-run` lists every file of every layer and the layer digests, and sends
nothing. It is the way to check patterns: it needs no registry and no commit,
and says which of those it could not work out instead of failing.

```
$ gitscale artefact publish --dry-run
Would publish ghcr.io/org/app/gitscale:9fceb02d0ae598e95dc970b74767f19372d61af8
  layer gitscale: 1 file, 312 B, sha256:e91d…
    .gitscale.toml
  layer vendor: 2 files, 1.1 KiB, sha256:4c1f…
    vendor/lib.js
    vendor/lib.css
  layer app: 3 files, 2.0 KiB, sha256:a90e…
    .well-known/security.txt
    app.js
    index.html
```

The image is a standard OCI image — one gzip tar layer per group and an image
config — so `docker`, `skopeo`, `crane` and `oras` read it, and an image another
tool pushed under the right name and tag installs like one gitscale published.

## Looking at what is published

Two read-only commands, run in the consuming workspace. Neither downloads an
image or records anything.

### artefact show

```
gitscale artefact show [-C DIR] [NAMES...]
```

For each artefact entry, everything there is to know right now: the image, the
commit the revision names on the remote, whether that commit is published and
with what layers, what is installed, and how the two compare. The one place to
answer *why does it say no artefact, missing or changed*.

```
meta/app
  revision   main
  image      ghcr.io/org/app/gitscale
  commit     3f2a9c1e5b7d4e8a9c217d4e5f6a8b90c1d2e3f4
  published  no — its pipeline may not have published this commit yet
  installed  9fceb02d0ae598e95dc970b74767f19372d61af8 (sha256:77d0…)
  status     behind, missing
```

A published commit lists its layers by group name, with size and digest. The
status compares what is installed with what the remote has *now* — unlike
[`gitscale status`](status.md), which compares it with what the last fetch saw:

| Status | Meaning |
|---|---|
| `ok` | Installed at the commit the revision names, with its current image |
| `not installed` | Nothing installed yet |
| `behind` | The revision names another commit now |
| `missing` | That commit has no image yet |
| `changed` | The installed commit's image was re-published with different files |
| `ref-mismatch` | Installed for a different revision than the config names |

An entry that cannot be looked up — no registry known, the remote unreachable,
access refused — shows an `error` line with the reason, the others are still
shown, and the command exits non-zero.

### artefact list

```
gitscale artefact list [-C DIR] [NAMES...]
```

Every commit an artefact entry has an image for in the registry, from its tag
list. Commits a branch or tag names now come first, labelled; the rest are
images of commits no ref points at any more. The installed one is marked:

```
meta/app  ghcr.io/org/app/gitscale  (3 images)
  3f2a9c1e5b7d4e8a9c217d4e5f6a8b90c1d2e3f4  main  (installed)
  9fceb02d0ae598e95dc970b74767f19372d61af8  v2.1.0
  1ccd849fb241a93202864043a528fa00d9b4279b
```

A full SHA from this list is what an entry's `revision` takes to pin an image
that no branch or tag names any more.

## Where artefacts live

The image is the repository's path under its registry, lowercased, with a
fixed `gitscale` suffix so artefacts stay apart from the images a project
publishes itself:

| Entry `url` | Image |
|---|---|
| `git@gitlab.com:org/sub/frontend.git` | `registry.gitlab.com/org/sub/frontend/gitscale` |
| `https://github.com/Org/Tools.git` | `ghcr.io/org/tools/gitscale` |
| `https://gitlab.corp.example/team/svc.git` | `registry.corp.example/team/svc/gitscale` (configured) |

The registry is found in this order:

1. `[registries]` in the config: a key containing `/` is a URL prefix
   (the longest that matches wins, and the image path is the rest of the URL);
   anything else is a host.
2. Inside a CI job, the CI server's own host maps to the registry it owns —
   `CI_REGISTRY` on GitLab, `ghcr.io` or `containers.<host>` on GitHub.
3. Built in: `github.com` → `ghcr.io`, `gitlab.com` → `registry.gitlab.com`.
4. Otherwise the entry fails, naming the setting to add.

```toml
[registries]
"gitlab.corp.example" = "registry.corp.example"
"/srv/repos/"         = "registry.corp.example:5000/mirrors"   # URL prefix, with a namespace
```

A registry on `localhost`, `127.0.0.1` or `[::1]` is spoken to over plain
HTTP, every other one over HTTPS — unless its value starts with `http://`, the
opt-in for a registry without TLS on a private network:

```toml
[registries]
"git.lab.internal" = "http://registry.lab.internal:5000"
```

No credential is ever sent over plain HTTP to anything but this machine: such
a registry is used anonymously.

## Logging in

A registry answers an anonymous request with a challenge naming a token
service; gitscale offers that service a credential, in this order:

1. **The CI job token**, in a GitLab or GitHub job — but only to the registry
   that CI server owns (`CI_REGISTRY`; `ghcr.io` or `containers.<host>`), and
   only through a token service on the CI server itself (GitLab's `/jwt/auth`)
   or on the registry (GHCR's `/token`). A registry that points anywhere else
   gets no credentials. As with git, the token is read from its variable when
   a request needs it, and never written anywhere.
2. **What `docker login` stored** for that registry: `~/.docker/config.json`
   (or `$DOCKER_CONFIG/config.json`), including credential helpers such as the
   macOS keychain or `pass`; then Podman's auth file
   (`$REGISTRY_AUTH_FILE`, `$XDG_RUNTIME_DIR/containers/auth.json`,
   `~/.config/containers/auth.json`). `podman login` and `oras login` write
   these too.
3. **Nothing**: a public package needs no credential.

On a developer machine that is one login per registry, with a token that can
read packages:

```
docker login ghcr.io                # a GitHub PAT with read:packages
docker login registry.gitlab.com    # a GitLab PAT or deploy token with read_registry
```

| | GitLab | GitHub |
|---|---|---|
| Credentials in a job | `gitlab-ci-token` + `CI_JOB_TOKEN` | `GITHUB_TOKEN` |
| Publishing | the job token pushes to its own project's registry | `permissions: packages: write` |
| Reading another project's | the consumer on the producer's job-token allowlist — the same one [cloning needs](ci-authentication.md) | the consumer repository added under the package's *Manage Actions access*, plus `permissions: packages: read` |
| Outside CI | PAT or deploy token with `read_registry` | PAT with `read:packages` |

A refusal names whichever of these is missing.

## Pipelines

### One-time setup

1. **Producer**: add an `[artefact]` table and a publish job that runs on
   every push to any branch consumers track. No `rules: changes`, `paths:`
   filters or skip conditions: a commit without an image breaks every consumer
   tracking that branch until the next build.
2. **Producer**: give consumers read access — GitLab's job-token allowlist,
   GitHub's *Manage Actions access*.
3. **GitLab producer**: exempt `*/gitscale` from the registry's cleanup policy,
   or pinned commits lose their images.
4. **Consumer**: declare the entry with `mode = "artefact"`.

### GitLab

Producer `.gitlab-ci.yml`:

```yaml
publish:
  stage: build
  script:
    - make dist
    - gitscale artefact publish
```

Consumer `.gitlab-ci.yml`:

```yaml
variables:
  GIT_CLEAN_FLAGS: -ffdx -e /meta/

test:
  script:
    - gitscale clone
    - make test
```

### GitHub

Producer workflow:

```yaml
on: push
permissions:
  contents: read
  packages: write
jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: make dist
      - run: gitscale artefact publish
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

Consumer workflow:

```yaml
permissions:
  contents: read
  packages: read
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: gitscale clone
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
      - run: make test
```

Publish on `push`, not `pull_request`: a pull request's `GITHUB_SHA` is a merge
commit no branch will ever name.

## The checkout

- **`clone`** resolves the revision, downloads every layer, checks each
  against its digest, unpacks them in order, and strips the write bits of every
  file at any depth.
- **`fetch`** resolves the revision and asks the registry whether that commit
  has an image, and records both. Nothing is downloaded, and no directory is
  created. A commit with no image is an error.
- **`pull`** does nothing — and asks the registry nothing — when the revision
  still names the installed commit. Otherwise it downloads the new image first
  and only then replaces the files, so a registry that fails part way leaves
  the installed version alone.
- **`push`** and **`commit`** skip artefact entries; [`clean`](clean.md) keeps
  them whole.

The directory holds the artefact's files and nothing else — dot files
included, all of them read-only, all of them replaced on update. GitScale
assumes nothing about their names. What it records about a checkout — the
revision, commit and image digest installed, and what the last fetch saw — is
kept outside it, in the workspace repository's git directory
(`.git/gitscale/artefacts/`, per worktree), or in `.gitscale/artefacts/` beside
the config when the workspace is not the top of a git repository. A checkout
deleted by hand is noticed as not installed, whatever was recorded.

An archive may not contain a path that leaves the directory, a symlink
pointing out of it, or anything but files, directories and symlinks.

Files are unpacked with the time of the download, not the archive's, so build
tools never take them for older than their own outputs.

## Status

`REF` shows the installed commit. The flags:

| Flag | Meaning |
|---|---|
| `ok` | Installed at the commit the last fetch saw |
| `behind` | The revision has moved to another commit since |
| `missing` | That commit has no image (yet) |
| `changed` | The installed commit's image was re-published with different files; `pull` installs it |
| `ref-mismatch` | Installed for a different revision than the config names now |

`--format json` adds an `artefact` object to the row, with the commit and
digest installed and the ones the last fetch saw.

## Caching

Images are kept per user in the [object cache](caching.md), beside the git
mirrors and snapshots: a second workspace, worktree or CI job on the same
machine downloads nothing, and a layer shared by several commits is stored
once. Refs still come from the remote — the commit from `ls-remote`, the image
digest from the registry — and every blob is checked against its digest when
it is read, so a stale or damaged cache changes how many bytes cross the wire,
never which files a checkout gets.

---

[← 2.8 Cleaning](clean.md) · [Contents](README.md) · [Next → 3. Configuration file reference](configuration.md)

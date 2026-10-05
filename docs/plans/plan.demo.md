# Plan: the GitScale demo

Status: agreed, not started. Needs
[plan.artefact-checkouts.md](plan.artefact-checkouts.md) and the
[GitScale changes](#gitscale-changes) below.

- [What it shows](#what-it-shows)
- [Repositories](#repositories)
- [Versions](#versions)
- [Images and artefacts](#images-and-artefacts)
- [CI](#ci)
- [Rendering the deployment](#rendering-the-deployment)
- [Local stack](#local-stack)
- [Demo script](#demo-script)
- [Bootstrap](#bootstrap)
- [GitScale changes](#gitscale-changes)

## What it shows

A small system of six repositories, every dependency pinned to a calver tag:

- a plain clone placing the whole system, and `git explain` on a dependency
  two repositories ask for;
- topics across repositories: join, commit, push dependencies first, the
  merge gate, promotion up the chain;
- a change to a shared library released to some of its users and not others;
- raising a dependency everywhere it is asked for;
- a developer taking dependencies as artefacts instead of sources, with no
  config change;
- the deployment rendered with the image tags of what is pinned, or, on a
  topic, of exactly the sources each service was built from;
- the stack running locally from any repository: its dependencies from their
  images, the repositories joined to the topic from their working trees, with
  hot reload.

## Repositories

All public, on GitHub under `thepartly`, cloned over HTTPS.

| Repository | Contents | Asks for |
|---|---|---|
| `gitscale-demo` | The workspace root: Argo CD applications and kustomize manifests, the render script, the demo script | application-a, application-b, frontend |
| `gitscale-demo-frontend` | Vite, React, TypeScript; two pages, one per application | application-a, application-b |
| `gitscale-demo-application-a` | axum server, `GET /api/a/hello`; `sdk/`: a handwritten TypeScript client | shared-libs |
| `gitscale-demo-application-b` | The same, `GET /api/b/hello` | shared-libs |
| `gitscale-demo-shared-libs` | Cargo workspace: `crates/logging` (tracing setup: service name, level, pretty or JSON), `crates/hello` (`greet(name, style)`) | — |
| `gitscale-demo-compose` | `compose` and `tags`, bash: see [local stack](#local-stack) | — |

```
gitscale-demo ──┬── application-a ──┐
                ├── application-b ──┴── shared-libs
                └── frontend ──┬── application-a
                               └── application-b
```

Every repository but shared-libs also asks for compose, at `imports/compose`;
it is left out of the drawing.

The root does not declare shared-libs: it arrives through the applications,
at `imports/shared-libs`. Pins are staggered so that `git explain` has
something to show: application-b asks for an older shared-libs than
application-a, and frontend for an older application-a than the root.

**Applications.** application-a calls `logging::init` pretty and
`greet(name, Plain)`; application-b JSON and `greet(name, Shout)`. Both depend
on the crates by path: `imports/shared-libs/crates/*`. Images are built in CI
only: in a workspace the application's `imports/shared-libs` is a link, which
`docker build` does not follow.

**SDK.** `sdk/` in each application is an npm package,
`@gitscale-demo/application-a-sdk`: `src/` exporting `hello(name)` and its
response type, built by `tsc` into `dist/` (ignored by git). Its
`package.json` points `types` and `import` at `dist/`.

**Frontend.** Pages `/a` and `/b` call `/api/a/hello` and `/api/b/hello` on
their own origin; both are proxied to `API_A_URL` and `API_B_URL` — by Vite
in development, by nginx in the image, through the official image's
templates, filled in at start. It depends on each SDK with
`"file:imports/application-a/sdk"`; `predev` and `prebuild` run the SDK's
build when its `dist/` is missing (a source checkout), and in development Vite
aliases each SDK to its `sdk/src`, so SDK edits reload too. The image is nginx
serving `dist/`, with a fallback to `index.html`. Frontend asks for the
applications with `recursive = false`: it needs their SDKs, not their
dependencies.

**Root.**

```
gitscale-demo/
├── .gitscale.toml
├── apps/<name>/            Deployment, Service, kustomization (image name only)
├── overlays/demo/          all three, an Ingress / → frontend, and frontend's API_A_URL, API_B_URL
├── argocd/                 one Application per service
├── render.sh               see Rendering the deployment
├── docker-compose.yml      none: the root's stack is its dependencies'
└── DEMO.md                 the demo script
```

**Locally.** The root at `~/projects/gitscale-demo`, the others as siblings
`~/projects/gitscale-demo-<name>`, each `git init` with no commits until
pushed. Once pushed, the root checks them out under its own `imports/`, and
the siblings can go. `demo/system` and its `.gitignore` lines are removed from
the gitscale repository.

**Repository settings**, for all six: squash merge only, automatically
delete head branches, and a ruleset on `main` requiring a pull request, the
`check` and `build` checks, and the branch up to date.

## Versions

Every merge to `main` tags `v1-YYYY.MM.DD-N` in UTC: `v1-2026.10.05-1`, then
`v1-2026.10.05-2` for the next merge that day. The major is written in the
tagging step of `release.yml`; a new major is a pull request changing it. See
[version tags](#1-version-tags-semver-and-calendar-versions-with-a-major).

## Images and artefacts

**Images:**

```
ghcr.io/thepartly/gitscale-demo/application-a
ghcr.io/thepartly/gitscale-demo/application-b
ghcr.io/thepartly/gitscale-demo/frontend
```

Each image is its own package, linked to the repository that pushes it, and
public.

| Tag | Meaning | Pushed by |
|---|---|---|
| `<hash>` | Built from exactly these sources: [`git scale hash`](#2-git-scale-hash), 64 hex digits | Every branch build |
| `v1-2026.10.05-1` | The release | `main`, added to the `<hash>` image |

A branch build whose `<hash>` exists skips building. `main` never builds: the
last branch pipeline, after `git upgrade --commit`, had the same tree and the
same pinned dependencies, so the release tag is added to its image with
`crane tag`.

**SDK artefacts.** Each application publishes its SDK as a GitScale artefact,
at the default `ghcr.io/thepartly/gitscale-demo-application-a/gitscale`, on
every build:

```toml
# application-a's .gitscale.toml
[[artefact.layer]]
name    = "sdk-dist"
include = ["sdk/dist/**"]

[[artefact.layer]]
name    = "sdk-src"
include = ["sdk/package.json", "sdk/tsconfig.json", "sdk/src/**"]
```

The application's own sources are not in it. Every workspace takes the
applications as sources unless its developer prefers otherwise
([plan.artefact-checkouts.md](plan.artefact-checkouts.md)).

## CI

Each repository carries its own workflows; none references another
repository's.

| Workflow | When | Jobs |
|---|---|---|
| `check.yml` | `pull_request` | `check`: `gitscale sync`, `gitscale check` |
| `build.yml` | `push`, every branch but `main` | `build`: `gitscale sync`, tests, the SDK artefact, then the image if `<hash>` is missing |
| `release.yml` | `push` to `main` | Check that the image and the SDK artefact of `<hash>` exist, else fail; tag the calver; `artefact publish --reuse`; `crane tag` the image. Nothing is built |

- `check` and `build` are separate jobs with no order between them, both
  required. Tests on a topic run against the topic's dependencies and give
  feedback; `check` keeps the pull request unmergeable until promotion, and
  promotion is a new commit, so everything runs again against the release
  tags.
- The tag and the image are one job: a tag pushed with `GITHUB_TOKEN` starts no
  other workflow. The check comes first, so a commit is never tagged without
  its image; a rerun reuses the version tag already on the commit.
- shared-libs has no image and no artefact: tests, then the tag. compose:
  `shellcheck`, then the tag.
- GitScale: `cargo install --git https://github.com/thepartly/gitscale --rev
  <sha> --locked`, cached by `<sha>`, until a release on crates.io has
  everything the demo needs; then `cargo install gitscale --locked`.
- The root's `build` runs `render.sh`, and its `release.yml` only tags.

## Rendering the deployment

`render.sh [--allow-released]` in the root, run by its CI on every push. The
tag of each service comes from `imports/compose/tags`, which the
[local stack](#local-stack) uses too. It never warns: a service either gets an
image of exactly its sources, or the render fails.

**On `main`**, every service gets its resolved calver tag.

**On a topic**, for each service:

1. Its tag is `git scale hash imports/<name>`: the sources its own CI
   builds at the commit it is at. No such image: fail,
   `application-a: no image for 3c9f…; push it, or wait for its pipeline`.
2. **A topic change it lacks** fails the render. Topic DEMO-3 changes
   shared-libs and joins application-a only:

   ```
                     shared-libs
                     on DEMO-3: changed
                      ▲           ▲
                      │           │
          application-a           application-b
          on DEMO-3: joined       at its pin: not joined
                │                       │
          its CI built it on      its CI built it on main,
          DEMO-3, with            with shared-libs
          shared-libs@DEMO-3      @v1-2026.10.01-1
                │                       │
          <hash> image            <hash> image = its release
          ✔ has the change        ✘ lacks the change
   ```

   ```
   application-b: lacks imports/shared-libs@DEMO-3 (built with v1-2026.10.01-1);
   join it, or render with --allow-released
   ```

   A dependency at another *pinned* version is not a topic change and never
   fails: application-b built with an older shared-libs than application-a
   asks for is a normal release.
3. **`--allow-released`**: a service that lacks topic changes gets its
   release tag instead of failing, and the rendered Deployment carries what it
   lacks, so it is in the output, not in a log:

   ```yaml
   metadata:
     annotations:
       gitscale-demo/lacks: "imports/shared-libs@DEMO-3"
   ```

   CI passes it when the root's pull request has the label
   `render:allow-released`.

Then `kustomize edit set image` on a copy of `overlays/demo`, `kustomize
build`, and print; the Argo CD Applications are printed too.

Inputs: `git scale hash --format json imports/<name>` (the hash, and each
source's directory and commit as its own build took it) and `git scale ls
--format json` (each checkout's resolved tag, commit, and whether it is on the
topic). Image existence: `crane manifest`.

## Local stack

Run from any repository of the workspace, or a clone of one:

```sh
cd imports/frontend
./imports/compose/compose up -d
```

**What `compose` does:**

1. **The repository** is the git top level of the current directory — not the
   script's own place, which is a link to the one checkout of compose.
2. **The stack** is that repository and its dependencies, recursively:
   the sources of `git scale hash --format json .`. Each is added once, in
   that order; one without `docker-compose.yml` (shared-libs, compose) adds no
   service.
3. **Local or image**, for each repository with a service:

   | Repository | Runs |
   |---|---|
   | On the topic here — joined, or the repository itself on a topic branch | Locally: its `x-local` command on the host, a forwarder in its place in the stack |
   | Any other | Its image, tagged by `tags` |
   | `--local DIR` | Locally, though not on the topic |
   | `--image DIR` | Its image, though on the topic — once its pipeline built what you pushed |

4. **Tags** from `tags` (below), for the repositories run as images; `local`
   for the ones run locally, whose image the forwarder replaces.
5. **Environment:** `GITSCALE_DEMO_<NAME>_DIR`, the absolute path of each
   repository in the stack, and `GITSCALE_DEMO_<NAME>_TAG` for each image;
   `<NAME>` is the repository name without `gitscale-demo-`, upper case,
   `-` as `_`. Compose files build every path from their own `_DIR`, so plain
   `-f` files combine whatever their order.
6. **`docker compose -p <repository name> -f … <arguments>`**: every argument
   not the script's own goes to compose, and with local repositories an
   override, generated on every run, comes after the repositories' files. `up`
   then runs detached, and each local command runs — one in the foreground, several
   together with their output prefixed by name. Ctrl-C stops them; the
   containers keep running until `compose down`. Other commands pass through
   with the same files.

**Options:** `--local DIR`, `--image DIR` (repeatable, relative to the current
directory), `--committed`, `--allow-released`.

**`tags [--committed] [--allow-released] DIR...`** prints
`GITSCALE_DEMO_<NAME>_TAG=<tag>` per repository:

- the repository's resolved calver tag when the workspace's root is on a
  pinned branch; otherwise `git scale hash` of it;
- **a topic change it lacks fails**: a repository not on the topic, run as its
  image, while a dependency of it is on the topic with changes, has an image
  without them:
  `application-b: lacks imports/shared-libs@DEMO-3 (built with v1-2026.10.01-1); join it, or --allow-released`.
  `--allow-released` gives it its release tag; the render also records that
  in an annotation;
- uncommitted changes in a repository run as an image fail:
  `imports/application-a has uncommitted changes; commit them, run it with --local, or --committed`.
  `--committed` hashes what is committed.

**A service repository** carries one file, `docker-compose.yml`: its service
as it runs from its image, and under the top-level `x-local` — which compose
ignores — how it runs on the host.

```yaml
# application-a/docker-compose.yml
x-local:
  application-a:
    run: cargo watch -w src -w imports/shared-libs -x run
    port: 18081

services:
  application-a:
    image: ghcr.io/thepartly/gitscale-demo/application-a:${GITSCALE_DEMO_APPLICATION_A_TAG:?}
    ports: ["8081:8080"]
```

```yaml
# frontend/docker-compose.yml
x-local:
  frontend:
    run: npm run dev -- --port 5173 --strictPort
    port: 5173
    env:
      API_A_URL: http://localhost:8081
      API_B_URL: http://localhost:8082

services:
  frontend:
    image: ghcr.io/thepartly/gitscale-demo/frontend:${GITSCALE_DEMO_FRONTEND_TAG:?}
    ports: ["8080:80"]
    environment:
      API_A_URL: http://application-a:8080
      API_B_URL: http://application-b:8080
```

| `x-local.<service>` | Meaning |
|---|---|
| `run` | The command on the host, in the repository's directory, in the foreground until killed |
| `port` | Where it listens; it gets it as `PORT` too |
| `env` | More environment for it: host processes reach the stack through `localhost` |

The script reads it with `docker compose config --format json` and `jq`. For
each service run locally, the override replaces its image with a forwarder,
under the same name, so the rest of the stack reaches the host process as it
would the container:

```yaml
services:
  application-a:
    image: alpine/socat
    command: tcp-listen:8080,fork,reuseaddr tcp-connect:host.docker.internal:18081
    extra_hosts: ["host.docker.internal:host-gateway"]
```

The container port (8080) comes from the service's `ports`, the host port
from `x-local.port`. Its `ports` stay, so a service keeps one public port on
localhost whichever way it runs:

| Service | Public | In the docker network | On the host, run locally |
|---|---|---|---|
| application-a | 8081 | `application-a:8080` | 18081 |
| application-b | 8082 | `application-b:8080` | 18082 |
| frontend | 8080 | `frontend:80` | Vite, 5173; hot reload through the forwarder |

The host needs Docker Compose v2, `jq`, `cargo-watch` and Node. A locally run
application compiles against the workspace's checkouts, uncommitted changes
included: a topic changing shared-libs runs with the change before anything
is pushed.

## Demo script

`DEMO.md` in the root. Every step uses the commands of
[the command line reference](../cli.md).

**0. Clone.** `git clone` the root; `git scale ls`;
`git explain imports/shared-libs` (two requests, the higher wins);
`git explain imports/application-a` (the root over frontend).

**Topics.** Each row is one topic, with the same steps: `git topic start`,
`git topic join` the repositories in the row, edit code in the ones it
changes, `git scale commit`, `git scale push`, `git topic status`; the pull
requests' `check` fails; merge and `git upgrade --commit` in the order `next
to merge` gives; `git topic finish`. The only files edited by hand are the
code changes. A repository joined without code changes is rebuilt against
the topic; its pins are written by `git upgrade --commit`. The "joined, no
code change" repositories are joined with `git topic join --dependants`, once
the level below carries a change, and `git topic leave` takes off the ones
the row leaves at their release.

| # | Code changed in | Joined, no code change | Left at its release | Render on the topic |
|---|---|---|---|---|
| 1 | application-a | — | application-b, frontend | frontend lacks application-a: label |
| 2 | application-a, frontend | — | application-b | passes |
| 3 | shared-libs | application-a | application-b, frontend | application-b lacks shared-libs, frontend lacks application-a: label |
| 4 | shared-libs | application-a, frontend | application-b | application-b lacks shared-libs: label |
| 5 | shared-libs | application-a, application-b | frontend | frontend lacks both: label |
| 6 | shared-libs, application-a, frontend | application-b | — | passes |

"Label" is `render:allow-released` on the root's pull request.

What promotion writes: shared-libs' tag into the configs of the joined
applications only — application-b keeps its older pin in 3 and 4, and `git
explain imports/shared-libs` shows application-a's request winning; an
application's tag into frontend's config when frontend is joined, and always
into the root's.

**7. Raise.** After 3: `git topic start DEMO-7-shared-libs`;
`git topic join --dependants imports/shared-libs` joins application-b, the one
asking for less; `git upgrade imports/shared-libs` raises its pin; push,
merge, `git upgrade --commit`.

**8. Artefacts.** In a clone of frontend:
`git scale prefer --artefact imports/application-a imports/application-b`,
`git scale pull`; `git scale ls` shows them as `artefact`, each holding `sdk/` and nothing of
the application, and frontend builds from the SDKs' `dist/`. `git scale
prefer --overlay imports/application-a`, `git scale pull`: the sources come back, with `dist/`
already built. Then topic 2 again: `git topic join imports/application-a`
gives a writable source checkout for the change; `git topic finish` returns it
to its preference. No file changes at any step.

**9. Local stack.** During topic 6, from `imports/frontend`:
`./imports/compose/compose up`. shared-libs, application-a and frontend are on
the topic: application-a and frontend run on the host with reload,
application-b from its image. An edit to `greet` in shared-libs restarts
application-a, and the page shows it. `--image imports/application-a` once
its pipeline has built the pushed commits. From the root,
`./imports/compose/compose up -d` with nothing joined: the whole system from
its release images.

## Bootstrap

Commits and pushes are done by hand; the plan stops at each point below.

1. Create the six repositories, apply the settings.
2. compose: push to `main`; CI tags it.
3. shared-libs: push to `main`; CI tags it. Push a second change: a second tag.
4. application-b pinning the first shared-libs tag, application-a the second;
   push both, twice for application-a so it has two tags.
5. frontend pinning the first application-a tag; push.
6. The root pinning the second application-a tag; push.
7. Make the three image packages and two artefact packages public.

Locally, the demo runs a GitScale built from the working tree; CI needs the
changes pushed to a branch of the gitscale repository, for `cargo install
--git`.

## GitScale changes

### 1. Version tags: semver, and calendar versions with a major

A tag is a version in exactly two forms, and nothing may come before them:

| Form | Grammar | Examples |
|---|---|---|
| Semver | `v` optional, then `MAJOR.MINOR.PATCH[-pre][+build]` | `v1.4.0`, `1.4.0`, `v2.0.0-rc.1` |
| Calendar | `v<major>-` required, then `YYYY.0M.0D` or `YYYY.0M.MICRO`, then an optional modifier | `v1-2026.10.05`, `v1-2026.10.05-1`, `v2-2026.11.02-rc1` |

- `<major>` is a number from 1, no leading zeros. The `v` is lowercase in
  both forms.
- Every other tag is not a version, and is decided by position:
  `api-v1.4.0`, `release-1.4.0`, `v2026.10.05`, `2026.10.05-2`,
  `api-v1-2026.10.05-1`.
- Streams go: there is no prefix to name one, so two versions of one form
  always compare.
- A calendar version's class is `Major(<major>)`, as a semver major: one
  checkout per major, `singleton` applies, a raise and a promotion stay within
  the major, and `upgrade --major` crosses it.
- A calendar version and a semver version never compare, as today.
- The modifier keeps today's order: a text modifier is a pre-release, before
  the bare date; a number is a later release that day, after it.

Code: `version.rs` — `parse` takes the whole tag: an optional `v` and semver,
or `v<digits>-` and a calendar version; `Version` loses `stream`, and
`compare` the stream check; `Version::class` gives a calendar version's major.
`promote::newest` loses its stream filter. `--major` needs no change beyond
the class.

Tests (`version.rs` unit tests, `resolution`, `upgrade`): both forms parse,
every rejected example doesn't; `v1-` and `v2-` are two slots; a raise stays
in `v1-`, `--major` reaches `v2-`; a prefixed tag is decided by position;
`prefixes_name_streams` goes; `edge_042` becomes a calendar major beside a
semver major.

Docs: `dependencies.md` revision kinds (*Streams* goes), and the
recommendation for producers: `vMAJOR.MINOR.PATCH` or `v1-YYYY.0M.0D-N`;
`recursive-dependencies.md` slots and *Comparing two requests* (no "same
stream"; the position error no longer names streams); every calver example in
`docs/`, `src/skill.md` and the plans.

### 2. `git scale hash`

```
git scale hash [GLOBAL] [--committed] [-f, --format text|json] [DIR...]
```

The hash of the sources a repository's own CI builds at the commit it is at,
for the root or each `DIR`. Text: `<hash>  <DIR>` per line, like `sha256sum`.
`ls --format json` carries it as `source_hash` on each checkout.

- **Which sources:** the repository's config resolved as if it were the root,
  from this workspace's stores: on a topic branch, with that topic's branches;
  at its pin or on a pinned branch, with pins only. That is what its own
  pipeline resolved, so its hash in its own CI and in the root's workspace
  agree, whatever else the root's workspace raised.
- **Input:** one line per repository reached, each once, sorted:
  `<normalised url> <tree id of its commit>`; for a checkout taken as an
  artefact, the tree id its image records. The repository itself is the first
  line, its URL from `origin`. SHA-256 of the lines, hex.
- Git's object ids, not file contents: nothing is read but refs and commits.
  A squash merge that changes no file keeps the tree, and the hash.
- Offline. A source with uncommitted changes fails:
  `imports/shared-libs has uncommitted changes; commit them, or --committed`.
  `--committed` hashes each source's commit, leaving its changes out.
- `--format json`: per `DIR`, the hash and each source's URL, the directory
  of its slot in this workspace, its commit, and its tree.

Tests: equal for a repository as the root and as a checkout; equal when the
root's workspace raised one of its dependencies; changes with a dependency's
tree, not with a commit that keeps the tree; follows the topic for a
repository on it and pins for one at its pin; `recursive = false` keeps the
entry's dependencies out; equal for a checkout taken as source and as
artefact; uncommitted changes fail, and `--committed` gives the hash of the
commit.

Docs: `cli.md`, `status.md` (`source_hash`).

### 3. `upgrade` deletes the promoted remote branch

A promoted slot's topic branch left on its remote keeps matching by name, so
the pipeline of the pin bump would still build from it. Promotion deletes it,
or fails.

1. **Check, before changing anything.** Fetch each promoted slot's store. For
   each whose remote has the topic branch, the tag must hold the remote tip's
   change (the content test promotion uses). One that doesn't — someone
   pushed after the merge — fails the whole `upgrade`, nothing changed:
   `imports/core: origin/feat/x has changes v1-2026.10.05-1 does not hold;
   merge or drop them, then run again`. A remote that cannot be reached fails
   the same way.
2. **Delete**, each as
   `git push --force-with-lease=refs/heads/<branch>:<checked tip> origin --delete <branch>`,
   so a push made since the check makes the deletion fail instead of losing
   it. A failed deletion — that, or no permission, or a protected branch —
   fails `upgrade`, with no config edited. Branches already deleted stay
   deleted: the tag holds their change. Running `upgrade` again continues.
3. **Then** edit the configs and, with `--commit`, commit. The pin bump
   exists only once no remote branch matches.

`--dry-run` lists what it would delete, and exits 1 when step 1 would fail.
There is no option to keep the remote branch.

Tests (`upgrade`): a held remote branch is deleted and the pin committed; a
remote branch with an extra commit fails with nothing changed; a push between
the check and the deletion fails the deletion and keeps the commit; a
rejected deletion leaves every config as it was; a second run after a partial
one completes; `--dry-run` lists deletions and changes nothing.

Docs: `topics.md` *Promotion*, `cli.md` `upgrade`.

### 4. `check`'s message

When a slot comes from a topic branch, `check` adds:
`hint: once released, git upgrade --commit and push; a branch deleted without
a new commit needs the whole pipeline run again`.

### 5. Remove `upgrade --resolved`

`git upgrade` keeps two forms: promote (no `DIR`) and raise (`DIR...`).
Writing what resolution selected into the root's config changes nothing that
is built, `git explain` already shows the winner, and implicit dependencies
are never written. A target other than the newest release, if one is ever
needed, is `git upgrade <DIR> --to <REV>`.

Code: `--resolved` in `src/lib.rs` and its path in
`src/commands/upgrade.rs`, with its refusals (`--major`, `-c`, not an entry of
the root).

Tests (`upgrade`): `normal_001`, `normal_009`, `normal_010`, `normal_011`
and `error_021b` go, and the `--resolved` cases of `error_020`; `edge_004`
moves to a raise, so table-style entries stay covered. Regenerate the test
catalog.

Docs: `topics.md` (*Writing what resolution selected* and its contents
line), `cli.md` `git upgrade`, `src/skill.md`.

### 6. `upgrade` edits only the topic; `git topic join --dependants`

Joining a topic happens only through `git topic join`, and a topic begins only
with `git topic start`.

**`git upgrade`**, both forms:

- Works on a topic only; off one: `not on a topic: git topic start NAME`.
- Edits the root's config and those of checkouts on the topic. Every other
  requester that asks for less is listed, not changed:
  `imports/application-b asks for v1-2026.10.01-1: git topic join imports/application-b`.
- `-c, --create` goes, with the topics a raise created
  (`upgrade/<name>-<tag>`, `upgrade/<date>`).

**`git topic join --dependants [DIR...]`** joins direct dependants: checkouts
whose config asks for the checkout in question. It never joins the root, and
otherwise joins as `git topic join` does, with the same refusals.

- **With `DIR`s:** the dependants of each that ask for less than its newest
  release in their major. The `DIR` need not be on the topic: this is how a
  raise begins.
- **Without:** one level up from the topic's changes. A checkout *carries a
  change* when it has commits on the topic that its pin does not, or was
  promoted on this topic. Take the carriers with a dependant not yet joined,
  keep those with no such carrier among their own dependencies — the lowest —
  and join their dependants. Then stop; run it again for the next level.
- A checkout joined with no commits carries no change until promotion commits
  its pin bump, so the climb follows the releases: frontend joins after
  application-a's pin moves to the new shared-libs, not before.
- **Left stays left:** `git topic leave` records the checkout as left for this
  topic, and `--dependants` passes over it; a plain `git topic join` of it
  clears the record. Promotion records what it promoted.
- Recorded in the root's git common dir, `gitscale/topics.toml`, per topic
  branch, by normalised URL; `git topic finish` drops the topic's record.
- Prints each checkout joined; with none, `nothing to join`, exit 0.

```sh
git topic join imports/shared-libs     # commits on the topic
git topic join --dependants            # application-a, application-b
git topic leave imports/application-b  # recorded: left on this topic
git topic join --dependants            # nothing: application-a carries no change yet
# shared-libs merged, tagged; git upgrade --commit bumps application-a's pin
git topic join --dependants            # frontend
```

Code: `src/commands/topic.rs` (`join --dependants`, the record in `leave` and
`finish`), `src/commands/upgrade.rs` (no joiners, no topic creation, no `-c`;
promotion records what it promoted; requesters off the topic listed), new
`src/topics.rs` for `topics.toml`.

Tests (`topic`, `upgrade`): with `DIR`, only dependants asking for less join;
without, one level from commits on the topic, and from a promotion; a joined
checkout without commits is not climbed from; the lowest carriers only; a
left checkout is passed over, and a plain join clears it; the root is never
joined; `finish` drops the record; `upgrade` off a topic fails; a requester off
the topic is listed with its join command and left unchanged; `-c` is gone.
`normal_005`, `normal_007`, `edge_017` and the `-c` cases of `error_020`
change from creating topics to listing requesters.

Docs: `topics.md` (*`git topic join` / `leave`*, *Promotion*, *Raising a
dependency*), `cli.md` (`git topic`, `git upgrade`), `src/skill.md`.

### 7. Tests for checkouts only dependencies ask for

- `git topic join` on an implicit checkout, including one taken as an artefact.
- Promotion of an implicit checkout edits the configs of every requester on
  the topic, and leaves a requester at its pin untouched.
- The root's pin wins over an older request from a requester left at its pin.

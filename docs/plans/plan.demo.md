# Plan: the GitScale demo

Status: agreed, not started. Needs
[plan.config-in-manifest.md](plan.config-in-manifest.md) for Argo CD to take
the root's artefact; every other GitScale feature it needs is implemented.

- [What it shows](#what-it-shows)
- [Repositories](#repositories)
- [Versions](#versions)
- [Images and artefacts](#images-and-artefacts)
- [CI](#ci)
- [Rendering the deployment](#rendering-the-deployment)
- [Environments](#environments)
- [Local stack](#local-stack)
- [Demo script](#demo-script)
- [Bootstrap](#bootstrap)

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
- each repository shipping its deployment and its environment variables with
  its code, so a topic changes both together;
- the deployment rendered with the image tags of what is pinned, or, on a
  topic, of exactly the sources each service was built from, and published as
  the root's artefact;
- the tags an Argo CD would follow: one per topic for its preview, and one
  for the latest release;
- the stack running locally from any repository: its dependencies from their
  images, the repositories joined to the topic from their working trees, with
  hot reload.

## Repositories

All public, on GitHub under `thepartly`, cloned over HTTPS.

| Repository | Contents | Asks for |
|---|---|---|
| `gitscale-demo` | The workspace root: the demo overlay, the render script, the Argo CD applications, the demo script | application-a, application-b, frontend |
| `gitscale-demo-frontend` | Vite, React, TypeScript; two pages, one per application | application-a, application-b |
| `gitscale-demo-application-a` | axum server, `GET /api/a/hello`, counting greetings in postgres; `sdk/`: a handwritten TypeScript client | shared-libs |
| `gitscale-demo-application-b` | The same, `GET /api/b/hello` | shared-libs |
| `gitscale-demo-shared-libs` | Cargo workspace: `crates/logging` (tracing setup: service name, level, pretty or JSON), `crates/hello` (`greet(name, style)`) | — |
| `gitscale-demo-compose` | `compose` and `tags`, bash; `toolchain/Dockerfile`: see [local stack](#local-stack) | — |

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
on the crates by path: `imports/shared-libs/crates/*`. application-a also
counts its greetings in postgres, at `DATABASE_URL` (sqlx, migrations run at
start), and replies with the count. Images are built in CI
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

**Deployment.** Each service repository ships how it is deployed, beside its
code, so a topic changing one changes the other in the same commits:

```
application-a/
├── deploy/
│   ├── kustomization.yaml      the two below; a ConfigMap generated from app.env
│   ├── deployment.yaml         image name only, port 8080, probes, envFrom the ConfigMap
│   ├── service.yaml            application-a:8080
│   ├── app.env                 DATABASE_URL=postgres://demo:demo@application-a-postgres:5432/demo
│   │                           RUST_LOG=info
│   └── components/postgres/    a kustomize Component: a StatefulSet and the Service application-a-postgres
└── docker-compose.yml          env_file: deploy/app.env
```

- **`app.env` is the one place a variable is written.** Kubernetes reads it
  through the ConfigMap, compose through `env_file`. Services are named the
  same in both, so even the addresses in it hold in both.
- Frontend's `app.env` holds `API_A_URL` and `API_B_URL`; application-b's,
  `RUST_LOG`.
- The demo has no secrets: the database password is in `app.env`.

**Root.**

```
gitscale-demo/
├── .gitscale.toml          its dependencies, and [artefact] include = ["rendered/**"]
├── overlays/demo/          the services' deploy/ bases, application-a's postgres component, an Ingress / → frontend
├── argocd/                 the demo Application and the topic ApplicationSet: see Environments
├── render.sh               see Rendering the deployment
├── rendered/               render.sh's output, ignored by git
├── docker-compose.yml      none: the root's stack is its dependencies'
└── DEMO.md                 the demo script
```

```yaml
# overlays/demo/kustomization.yaml
resources:
  - ../../imports/application-a/deploy
  - ../../imports/application-b/deploy
  - ../../imports/frontend/deploy
  - ingress.yaml                      # host demo.example.com
components:
  - ../../imports/application-a/deploy/components/postgres
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

Every merge to `main` is released as `v1-YYYY.MM.DD-hhmmss` in UTC:
`v1-2026.10.05-153012`. The major is written in the release step of
`release.yml`; a new major is a pull request changing it. See
[revision kinds](../dependencies.md#revision-kinds).

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
| `<hash>` | Built from exactly these sources: [`git scale hash`](../cli.md#git-scale-hash), 64 hex digits | Every branch build |
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
([artefacts](../artefacts.md#choosing-how-a-checkout-arrives)).

**The root's artefact** is the rendered deployment, `rendered/**` in one
group, at `ghcr.io/thepartly/gitscale-demo/gitscale`. The root's source hash
covers every repository's tree, so the artefact under it is the deployment of
exactly those sources. One group makes it a single layer, which Argo CD takes
as an OCI source.

| Tag | Meaning | Pushed by |
|---|---|---|
| `<hash>` | The render of exactly these sources | Every branch build of the root |
| `topic-<branch slug>` | Moves to the branch's latest `<hash>`: what its preview runs | Every branch build of the root |
| `v1-2026.10.05-153012` | The release | `main`, with `--reuse` |
| `released` | Moves to the latest release: what the demo environment runs | `main` |

The moving tags are not versions, so workspaces never see them.

## CI

Each repository carries its own workflows; none references another
repository's.

| Workflow | When | Jobs |
|---|---|---|
| `check.yml` | `pull_request` | `check`: `gitscale sync`, `gitscale check` |
| `build.yml` | `push`, every branch but `main` | `build`: `gitscale sync`, tests, the SDK artefact, then the image if `<hash>` is missing |
| `release.yml` | `push` to `main` | Name the release — the version already on the commit, else `v1-<UTC date>-<time>`; `crane tag` the image of `<hash>` with it, failing when there is none; `artefact publish --reuse <release>`, which releases the SDK artefact of `<hash>`; then the git tag, pushed. Nothing is built |

- `check` and `build` are separate jobs with no order between them, both
  required. Tests on a topic run against the topic's dependencies and give
  feedback; `check` keeps the pull request unmergeable until promotion, and
  promotion is a new commit, so everything runs again against the release
  tags.
- The tag and the image are one job: a tag pushed with `GITHUB_TOKEN` starts no
  other workflow. The git tag comes last, so a commit is never released
  without its images; a rerun reuses the version tag already on the commit.
  Release jobs run one at a time (`concurrency: release`).
- shared-libs has no image and no artefact: tests, then the tag. compose:
  `shellcheck`, then the tag.
- GitScale: `cargo install --git https://github.com/thepartly/gitscale --rev
  <sha> --locked`, cached by `<sha>`, until a release on crates.io has
  everything the demo needs; then `cargo install gitscale --locked`.
- The root's `build` runs `render.sh`, `artefact publish` (under `<hash>`)
  and `crane tag` to `topic-<branch slug>`. Its `release.yml` runs
  `artefact publish --reuse <release>` and `crane tag` to `released`. The
  last branch build, after `git upgrade --commit`, rendered with every service
  at its release, so the release is that render.

## Rendering the deployment

`render.sh [--allow-released]` in the root, run by its CI on every branch
push. The tag of each service comes from `imports/compose/tags`, which the
[local stack](#local-stack) uses too. It never warns: a service either gets an
image of exactly its sources, or the render fails.

**At its pin**, a service gets its resolved calver tag; on `main`, and after
promotion on a topic, every service is.

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

Then `kustomize edit set image` on a copy of `overlays/demo`, and `kustomize
build` into `rendered/demo/`: `manifests.yaml`, and a `kustomization.yaml`
listing it, so each [environment](#environments) can patch it. The output
names no namespace: the environment chooses it.

The label never changes a render that succeeds: without it, a service lacking
a topic change fails the render, and where nothing lacks one, it has no
effect. So each hash has one render, and `artefact publish` never finds two.

Inputs: `git scale hash --format json imports/<name>` (the hash, and each
source's directory and commit as its own build took it) and `git scale ls
--format json` (each checkout's resolved tag, commit, and whether it is on the
topic). Image existence: `crane manifest`.

## Environments

The demo deploys nothing and touches no cluster: it produces the artefacts
and moves their tags. An Argo CD 3.1 or later, wherever it runs, deploys the
root's artefact as an OCI source, path `rendered/demo`, following one of its
moving tags; CI holds no Argo CD credentials. `argocd/` holds what that Argo
CD would be given. The demo never applies it.

| Environment | `argocd/` | Follows | Namespace, host |
|---|---|---|---|
| demo | an Application | `released` | `demo`, `demo.example.com` |
| A preview per topic | an ApplicationSet, one Application per open pull request of the root | `topic-<branch slug>` | `topic-<number>`, `topic-<number>.example.com` |

```yaml
# argocd/topics.yaml
apiVersion: argoproj.io/v1alpha1
kind: ApplicationSet
metadata: { name: gitscale-demo-topics, namespace: argocd }
spec:
  goTemplate: true
  generators:
    - pullRequest:
        github: { owner: thepartly, repo: gitscale-demo }
        requeueAfterSeconds: 60
  template:
    metadata: { name: 'topic-{{.number}}' }
    spec:
      project: default
      source:
        repoURL: oci://ghcr.io/thepartly/gitscale-demo/gitscale
        targetRevision: 'topic-{{.branch_slug}}'
        path: rendered/demo
        kustomize:
          patches:
            - target: { kind: Ingress }
              patch: |-
                - op: replace
                  path: /spec/rules/0/host
                  value: topic-{{.number}}.example.com
      destination:
        server: https://kubernetes.default.svc
        namespace: 'topic-{{.number}}'
      syncPolicy:
        automated: { prune: true }
        syncOptions: [CreateNamespace=true]
```

- The root's `build` makes the slug as the generator does (`branch_slug`), so
  the tag it moves is the one the preview follows.
- A preview would have its own postgres, from application-a's component, and
  go with its namespace when the pull request closes.
- Every push to the root's topic branch moves its tag. A push to a service
  alone starts no build of the root: re-run the root's latest `build`, which
  resolves the topic again and publishes under the new hash.

## Local stack

Run from any repository of the workspace, or a clone of one:

```sh
cd imports/frontend
./imports/compose/compose up -d
```

Every service runs in the stack's network under its own name, from its image
or from its working tree. A service and its environment are written once, in
its repository's `docker-compose.yml`; running it from the working tree
changes only how it runs.

**What `compose` does:**

1. **The repository** is the git top level of the current directory — not the
   script's own place, which is a link to the one checkout of compose. **The
   workspace** is the outermost repository containing it; a clone is its own.
2. **The stack** is that repository and its dependencies, recursively:
   the sources of `git scale hash --format json .`. Each is added once, in
   that order; one without `docker-compose.yml` (shared-libs, compose) adds no
   service.
3. **Image or working tree**, for each repository with a service:

   | Repository | Runs |
   |---|---|
   | On the topic here — joined, or the repository itself on a topic branch | From its working tree: `x-build.run` in the [toolchain](#the-toolchain) container |
   | Any other | Its image, tagged by `tags` |
   | `--build DIR` | From its working tree, though not on the topic |
   | `--image DIR` | Its image, though on the topic — once its pipeline built what you pushed |

4. **Tags** from `tags` (below), for the repositories run as images.
5. **Environment:** `GITSCALE_DEMO_<NAME>_DIR`, the absolute path of each
   repository in the stack, and `GITSCALE_DEMO_<NAME>_TAG` for each image;
   `<NAME>` is the repository name without `gitscale-demo-`, upper case,
   `-` as `_`. Compose files build every path from their own `_DIR`, so plain
   `-f` files combine whatever their order.
6. **`docker compose -p <workspace name> -f … <arguments>`**: the
   repositories' files, then the [override](#the-override), generated on
   every run into the repository's git directory. The project is named after
   the workspace, so it is one stack whichever repository starts it. Every
   argument not the script's own goes to compose.

**Options:** `--build DIR`, `--image DIR` (repeatable, relative to the current
directory), `--network NAME`, `--committed`, `--allow-released`.

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
  `imports/application-a has uncommitted changes; commit them, run it with --build, or --committed`.
  `--committed` hashes what is committed.

### A service repository

It carries one file, `docker-compose.yml`: its services as they run from
images, and under the top-level `x-build` — which compose ignores — how each
runs from the working tree.

```yaml
# application-a/docker-compose.yml
x-build:
  application-a:
    run: cargo watch -w src -w imports/shared-libs -x run
    environment:
      RUST_LOG: debug

services:
  application-a:
    image: ghcr.io/thepartly/gitscale-demo/application-a:${GITSCALE_DEMO_APPLICATION_A_TAG:?}
    ports: ["8081:8080"]
    env_file: ${GITSCALE_DEMO_APPLICATION_A_DIR}/deploy/app.env
    depends_on:
      application-a-postgres: { condition: service_healthy }

  application-a-postgres:
    image: postgres:17
    environment: { POSTGRES_USER: demo, POSTGRES_PASSWORD: demo, POSTGRES_DB: demo }
    healthcheck: { test: pg_isready -U demo, interval: 1s }
    volumes: ["application-a-postgres:/var/lib/postgresql/data"]

volumes:
  application-a-postgres:
```

```yaml
# frontend/docker-compose.yml
x-build:
  frontend:
    run: "[ -d node_modules ] || npm ci; npm run dev -- --host 0.0.0.0 --port 80 --strictPort"

services:
  frontend:
    image: ghcr.io/thepartly/gitscale-demo/frontend:${GITSCALE_DEMO_FRONTEND_TAG:?}
    ports: ["8080:80"]
    env_file: ${GITSCALE_DEMO_FRONTEND_DIR}/deploy/app.env
```

A service's variables come from its `deploy/app.env`, the file Kubernetes
reads too; `environment` in `docker-compose.yml` only for what differs
locally, which wins over the file.

| `x-build.<service>` | Meaning |
|---|---|
| `run` | The command, in the repository's directory, in the foreground until stopped. It listens where the image does: the container port of the service's `ports` |
| `environment` | Added to the service's; a variable of the same name is replaced |
| `resources` | `cpus` and `memory`, replacing the service's limits; the build runs within them |

The script reads it with `docker compose config --format json` and `jq`.

### The override

For each service run from its working tree, the override replaces how it
runs. Compose merges the rest — its name, `ports`, `env_file`,
`depends_on` — from the service's own file:

```yaml
services:
  application-a:
    image: gitscale-demo-toolchain:3f9c…
    build: /home/me/projects/gitscale-demo/imports/compose/toolchain
    user: "1000:100"
    working_dir: /home/me/projects/gitscale-demo/imports/application-a
    entrypoint: []
    command: [bash, -c, "cargo watch -w src -w imports/shared-libs -x run"]
    restart: "no"
    environment:
      RUST_LOG: debug
      CARGO_TARGET_DIR: target/compose
    volumes:
      - /home/me/projects/gitscale-demo:/home/me/projects/gitscale-demo
      - /home/me:/home/me
      - /etc/passwd:/etc/passwd:ro
      - /etc/group:/etc/group:ro
```

- The workspace is mounted at its own path: the links between checkouts are
  relative inside it, and the worktrees' `.git` files name absolute paths.
- `$HOME` is mounted for the caches in it, cargo's registry and npm's. The
  user is yours, with its `/etc/passwd` entry, so what the build writes is
  yours.
- The rest of the stack reaches the service as `application-a:8080` either
  way, and it reaches `application-a-postgres` by name.
- The build's output, and the restarts after each edit, are the service's
  log: `compose logs -f application-a`, or `compose up` without `-d`.

The override also always carries **`shell`**: the toolchain with nothing to
run, in the current directory, on the stack's network.
`compose run --rm shell` gives a shell where every service answers by name.

| Service | Public | In the network |
|---|---|---|
| application-a | 8081 | `application-a:8080` |
| application-a-postgres | — | `application-a-postgres:5432` |
| application-b | 8082 | `application-b:8080` |
| frontend | 8080 | `frontend:80`; Vite's hot reload through the same port |

### The toolchain

`toolchain/Dockerfile` in compose: Rust, cargo-watch and Node, installed
outside `$HOME`. Built on first use as `gitscale-demo-toolchain`, tagged by
the hash of `toolchain/`. `CARGO_TARGET_DIR` is `target/compose`, so a build
in the stack and one outside it keep their own.

### From a dev container

The paths in the override are read by the Docker daemon, on its host. A dev
container using the host's daemon works when it has the workspace and `$HOME`
at the same paths as the host, and the host's `/etc/passwd` and `/etc/group`.

`--network NAME` runs the stack on an existing network instead of its own, as
the override's `default` network, external. A dev container on that network
reaches every service by name, as `shell` does.

The host needs Docker Compose v2 and `jq`. An application run from its working
tree compiles against the workspace's checkouts, uncommitted changes included:
a topic changing shared-libs runs with the change before anything is pushed.

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

In topic 2, application-a also reads `GREETING_PREFIX`, added to its
`deploy/app.env` in the same commit as the code: the local stack and the
topic's preview both get it from that one line.

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
the application, and frontend builds from the SDKs' `dist/`. Then topic 2 again: `git topic join
imports/application-a` gives a writable source checkout for the change; `git
topic finish` returns it to its preference. No file changes at any step.

**9. Local stack.** During topic 6, from `imports/frontend`:
`./imports/compose/compose up -d`. shared-libs, application-a and frontend are
on the topic: application-a and frontend run from their working trees with
reload, application-b and application-a's postgres from their images. An edit
to `greet` in shared-libs restarts application-a, and the page shows it, with
the count carried over. `--image imports/application-a` once its pipeline has
built the pushed commits. From the root,
`./imports/compose/compose up -d` with nothing joined: the whole system from
its release images.

**10. Deployment artefacts.** With `crane` and `kustomize`, during topic 6,
after the root's push, with `R=ghcr.io/thepartly/gitscale-demo/gitscale`:

- `crane manifest $R:topic-<slug>`: one layer, and `dev.gitscale.hash` equal
  to `git scale hash` in the root;
- `crane export $R:topic-<slug> - | tar -x`, then `kustomize build
  rendered/demo`: every service at the image of exactly its sources, what a
  preview would run;
- another push to the root: `topic-<slug>` moves to the new hash.

After the merges and `git upgrade --commit`: `crane ls $R` shows the release,
and `crane digest` gives the same digest for it, for `released`, and for the
topic's last render. Nothing was rendered twice.

## Bootstrap

Commits and pushes are done by hand; the plan stops at each point below. A
repository that publishes — the applications, frontend and the root — pushes
a branch first, whose build publishes under the hash, then `main`, which
releases that with `--reuse`.

1. Create the six repositories, apply the settings.
2. compose: push to `main`; CI tags it.
3. shared-libs: push to `main`; CI tags it. Push a second change: a second tag.
4. application-b pinning the first shared-libs tag, application-a the second;
   push both, twice for application-a so it has two tags.
5. frontend pinning the first application-a tag; push.
6. The root pinning the second application-a tag; push.
7. Make the three image packages and the three artefact packages (two SDKs,
   the root's render) public.

Locally, the demo runs a GitScale built from the working tree; CI needs the
changes pushed to a branch of the gitscale repository, for `cargo install
--git`.

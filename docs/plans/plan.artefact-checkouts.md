# Plan: how a checkout arrives — source, artefact or overlay

Status: agreed, not started. Needs `git scale hash` from
[plan.demo.md](plan.demo.md#2-git-scale-hash).

- [The idea](#the-idea)
- [Workflows](#workflows)
- [Specification](#specification)
- [Implementation](#implementation)

## The idea

A config says **what** a repository depends on: the repository and its
revision. **How** each checkout arrives in a workspace — its sources, its
published artefact, or its sources with the artefact laid over them — is the
workspace's choice, made per dependency, never written in a config:

- a frontend developer who never edits the SDKs takes them as artefacts:
  fewer files, faster placement, nothing of the services in their searches;
- a developer with no access to a repository's sources gets its artefact,
  with nothing to configure;
- whoever develops a repository has its sources, by joining the topic.

Every repository a workspace checks out is one checkout per major, whatever
form it arrives in.

## Workflows

```sh
# Every checkout is its sources until you say otherwise
git clone https://github.com/acme/app.git && cd app

# Take the SDKs as artefacts
git scale prefer --artefact imports/billing-sdk imports/users-sdk

# Keep the sources, with the build already in place
git scale prefer --overlay imports/core

# See the preferences
git scale prefer
#   imports/billing-sdk   artefact
#   imports/users-sdk     artefact
#   imports/core          overlay

# Back to the sources
git scale prefer --source imports/core

# Apply them: prefer only records, the next placement applies
git scale pull

# Develop one: its sources, writable, whatever it was
git topic join imports/billing-sdk
# Done with it: back to its preference
git topic leave imports/billing-sdk

# No access to a repository's sources: its artefact, automatically
git scale ls    # imports/vault-client  artefact  (no access to sources)
```

CI behaves the same: sources unless a job runs `git scale prefer` first.

## Specification

### Forms

| Form | The directory holds |
|---|---|
| `source` | A worktree of the repository's store |
| `artefact` | The image of the commit, and nothing else, read-only |
| `overlay` | A source checkout with the image of its commit laid over it |

### Which form a checkout gets

The first that applies:

1. **Joined to the topic here:** `source`, writable. With an `overlay`
   preference, the image of its commit laid over it while there is one.
2. **Its sources cannot be read, and the image can:** `artefact`.
3. **Its preference**, `artefact` or `overlay`. The repository must publish
   artefacts — `[artefact]` in its config at the resolved commit — and the
   commit must have an image. Otherwise placement of that checkout fails:
   `imports/core prefers artefact, but 3f2a9c1 has no image; or: git scale
   prefer --source imports/core`. The same for a topic branch followed without
   being joined (a colleague's): the image of its tip, never an older one.
4. **`source`.**

### `git scale prefer`

```
git scale prefer [GLOBAL] [DIR...]
git scale prefer [GLOBAL] --source|--artefact|--overlay <DIR>...
```

- Without a form: the preferences of the `DIR`s, or every preference, with
  the checkout each applies to.
- With a form — exactly one of the three, and at least one `DIR`: the
  preference for the repository of each checkout, all its majors, wherever it
  is checked out. `--source` removes it.
- Stored in the root's git common dir, `gitscale/prefer.toml`, so every
  worktree of the root shares it; keyed by normalised URL:

  ```toml
  [repos]
  "github.com/acme/core" = "overlay"
  ```

- Works in CI as anywhere else.
- It only records. The next placement — `git scale pull`, or the hook on the
  next switch or pull — applies it; until then `ls` shows the checkout's form
  and the one it will take, `source → artefact`. A checkout holding work is
  kept by that placement, as every placement keeps one.

### No access to a repository's sources

Rule 2 is taken when `git ls-remote` fails with an authentication or
not-found error (not a network error), and the registry answers. Then the
registry stands in for git:

| Needs | From |
|---|---|
| The versions there are | The registry's tag list, read as [revision kinds](../dependencies.md#revision-kinds) |
| Which commit a version names | The image tagged with it: its `org.opencontainers.image.revision` |
| The checkout's own dependencies | The image's `gitscale` layer |
| Its tree, for `git scale hash` | The image's `dev.gitscale.tree` annotation |

A branch revision needs the sources:
`imports/vault-client asks for main, which needs access to its sources`. On a
topic, a repository whose sources cannot be read stays at its pin. Neither
sources nor image readable: the git error, and the registry's.

### Publishing

Every image is tagged with its commit SHA and with the
[source hash](plan.demo.md#2-git-scale-hash) of what it was built from, and
carries `dev.gitscale.tree`. `artefact publish` also tags it with every
version tag on the commit (`git tag --points-at`).

**Release without rebuilding.** A squash merge gives `main` a new commit, with
the same sources as the branch's last build, which already published.

```
gitscale artefact publish --reuse [-C DIR] [--commit SHA]
```

- An image tagged with the source hash of this commit exists: push a manifest for this
  commit — the same layers, `revision` this commit — tagged with the commit
  SHA and every version tag on it. No file is packed, no layer uploaded.
- None: fail, `no image of these sources (3c9f…)`.
- `--dry-run`: says which, and changes nothing.

A release pipeline builds nothing: `artefact publish --reuse --dry-run`
first, so a missing image fails it before the version is tagged; then the
tag; then `artefact publish --reuse`.

### Resolution

A slot is keyed by repository and major. Form takes no part in resolution:
every request for a repository is one request, whatever form each workspace
gives it.

### Promotion

In order: the tag holds the change (`not tagged yet` until then); then, when
the dependency publishes artefacts — `[artefact]` in its config at the tag —
the tag's commit has an image (`tagged TAG, no image yet` until then).

### `ls`

The `ARTEFACT` column becomes `AS`: `source`, `artefact`, `overlay`. JSON:
`as`, `as_reason` (`joined`, `preferred`, `no-access`, or `default`), and
`as_next`: the form the next placement gives, when it differs.

## Implementation

### Code

| Where | Change |
|---|---|
| `config.rs` | Remove `artefact` from entries, with `ArtefactUse`, `is_artefact`, `is_overlay` |
| `resolution.rs` | `SlotKey` loses `kind`; `Kind::of` and the `_artefact` naming go; `Slot.artefact` becomes the form, decided after resolution |
| new `src/prefer.rs` | `prefer.toml`: read, write; the form rules above |
| new `src/commands/prefer.rs` | The command |
| `checkout.rs` | Place by form; switch a checkout between forms, keeping one that holds work |
| `stores.rs`, `registry.rs` | Rule 2: tell an access failure from a network failure; versions, dependencies and tree from the registry |
| `artefact.rs`, `commands/artefact.rs` | Source hash tag, `dev.gitscale.tree`, version tags; `publish --reuse` |
| `promote.rs` | The image wait follows the dependency's `[artefact]` |
| `commands/require.rs` | Remove `--artefact` |
| `commands/ls.rs` | `AS`, `as`, `as_reason` |
| `commands/topic.rs` | `join` gives a source checkout from any form; `leave` returns to the preference |

### Tests

- **forms:** `prefer --artefact DIR` gives the image, `prefer --source` the
  sources again; `overlay` lays the image over; a preference applies to every
  major and survives a move of the checkout's directory; every worktree of the
  root sees it.
- **failures:** a preference with no image for the commit, or for a repository
  without `[artefact]`, fails that checkout's placement with the hint; a
  followed topic branch without an image of its tip fails the same way.
- **records only:** `prefer` changes no checkout; `ls` shows `as_next`;
  `git scale pull` applies it.
- **work kept:** a source checkout with changes is not replaced.
- **topic:** `join` on an artefact gives a writable source checkout at the
  same commit; `leave` restores the artefact.
- **no access:** sources refused and image readable give `artefact`, at the
  commit a version tag names in the registry, with its dependencies from the
  `gitscale` layer; a branch revision fails naming access; on a topic it stays
  at its pin; a network failure is not taken for no access.
- **publish:** the source hash tag, tree annotation and version tags; `--reuse`
  after a squash merge tags a new manifest of the same layers for the new
  commit and uploads no layer; `--reuse` with no image of these sources fails.
- **resolution:** requests from workspaces with different preferences give
  one checkout.
- **promotion:** waits for the tag, then for the image only for a dependency
  with `[artefact]`.
- **hash:** equal for a checkout as `source` and as `artefact`.
- **existing:** artefact tests move from config entries to `prefer`.
  Regenerate the test catalog.

### Docs

- `artefacts.md`: *Declaring an artefact entry* becomes *Choosing how a
  checkout arrives*; *The checkout*, *Overlay*, *An artefact on a topic* and
  *Status* follow the forms; publishing gains the tags, the annotation and
  `--reuse`, and *Pipelines* the release without rebuilding; a section on no
  access.
- `dependencies.md`: *Artefacts* points to `prefer`; no `artefact` in the
  entry examples.
- `configuration.md`: no `artefact` in `[repos]`.
- `recursive-dependencies.md`: slots are repository and major; *Where they
  go* and *Source or artefact* have no artefact cases.
- `cli.md`: `prefer`; `require` without `--artefact`.
- `status.md`: `AS`, `as`, `as_reason`.
- `topics.md`: `join` and `leave` with forms.
- `src/skill.md`, `README.md`.

# Plan: Windows support

Status: proposed, not started. GitScale builds and runs on Linux and macOS
only; this plan covers what it takes to build, test and support it on Windows.

- [Why it does not build today](#why-it-does-not-build-today)
- [The one design question: dependency links](#the-one-design-question-dependency-links)
- [Everything else](#everything-else)
- [Testing and CI](#testing-and-ci)
- [Phases](#phases)
- [Open questions](#open-questions)

## Why it does not build today

Unix-only APIs are used without a `cfg` gate, so the crate does not compile for
a Windows target at all:

| Where | What | Unix assumption |
|---|---|---|
| `src/resolve.rs`, `create_symlinks` | Dependency links | `std::os::unix::fs::symlink`, relative targets |
| `src/git.rs`, `set_write_bits` | Readonly checkouts | `std::os::unix::fs::PermissionsExt` mode bits |
| `src/artefact.rs`, `set_write_bits` and extraction | Readonly artefacts, executable bits | `PermissionsExt` |
| `src/hooks.rs`, `run_post_sync` | `[hooks] post_sync` | `Command::new("sh")` |
| `src/commands/hook.rs` | Installed git hook shim | `#!/bin/sh` script; `make_executable` is already `cfg(unix)`-gated |
| `src/commands/hook.rs`, `Scope::System` | `hook install --system` | `/etc/gitscale` |
| `src/cache.rs`, `resolve_dir` | Default CI cache location | `XDG_DATA_HOME`, then `HOME/.local/share` — on Windows `HOME` is usually unset, so the cache silently turns off |
| `src/registry.rs`, stored credentials | `docker login` / Podman credentials | `HOME/.docker/config.json`, `XDG_RUNTIME_DIR`, `HOME/.config/containers/auth.json` |
| Unit tests in `src/artefact.rs` and others | Fixtures | `std::os::unix::fs::symlink`, `#!/bin/sh` scripts |

Already portable: the cache lock (`File::lock`), git invocation, HTTP, tar and
gzip, TOML, path handling through `Path` in most places.

## The one design question: dependency links

GitScale plants a relative symlink at each path where a repository declares a
dependency, pointing at the one checkout of it (`imports/b/libs/d → ../../d`).
Relative links are what let a workspace be moved or copied without breaking.

On Windows:

| Mechanism | Needs | Relative | Notes |
|---|---|---|---|
| Directory symlink (`std::os::windows::fs::symlink_dir`) | Developer Mode, or `SeCreateSymbolicLinkPrivilege` (admin) | Yes | Same behaviour as Unix when allowed |
| Directory junction | Nothing | **No** — absolute target, same volume | Not in `std`: `junction` crate, or `mklink /J` |
| Copy | Nothing | n/a | Defeats the purpose: two copies, no single checkout |

**Proposal:** try `symlink_dir`; when it fails with a privilege error, fall
back to a junction and say so once per command (`links are junctions: enable
Developer Mode for relative links`). Consequences to handle:

- **Moving a workspace** breaks junctions. `status` must report a junction
  whose target is outside the workspace (or missing) as needing a relink, and
  `sync` must recreate it — today's orphan/unlinked logic assumes relative
  links (`classify_orphan`, `is_outer_link`, `lexical_join` in `src/resolve.rs`).
- **Detection.** `is_symlink()` and `read_link()` behave differently for
  junctions; every place that asks "is this one of our links" (`resolve.rs`,
  `status.rs`, `clean.rs`, `sync.rs`, the `untracked-links` check, the ledger)
  needs one shared helper that recognises both kinds.
- **git sees them differently.** A junction is a directory to git; a symlink is
  a file. The `untracked-links` flag and `is_tree_modified` (which ignores
  untracked symlinks) must treat a junction the same way, or every dependant
  reads as dirty.
- **Removal.** Removing a junction must never recurse into its target:
  `remove_dir` on the junction, never `remove_dir_all`. Audit every removal of
  a link (`sync` relink and orphan removal, `pull` replacing a symlink).

## Everything else

| Area | Change |
|---|---|
| Readonly checkouts and artefacts | `Permissions::set_readonly(true/false)` on Windows instead of mode bits; skip `.git` as today. Keep one `set_write_bits` per platform behind `cfg` |
| Executable bits in artefacts | Not meaningful on Windows; extract without them, `publish` keeps writing them from Unix producers |
| `post_sync` hook | Run through Git for Windows' `sh` when found (`git --exec-path`/`../../bin/sh.exe`), else `cmd /C`. Document which |
| Hook shim | Git for Windows runs hooks with its bundled `sh`, so the `#!/bin/sh` shim works; verify `core.hooksPath` installs, path quoting with spaces and backslashes in the baked-in binary path |
| `hook install --system` | `%ProgramData%\gitscale` instead of `/etc/gitscale` |
| Cache location | `GITSCALE_CACHE_DIR`, else `%LOCALAPPDATA%\gitscale` (use `dirs::data_local_dir`) |
| Registry credentials | `%USERPROFILE%\.docker\config.json` (Docker Desktop) and its `credsStore` (`wincred`/`desktop`) helpers |
| Paths in records | Ledger keys, artefact marker slugs and link paths are written with `/` from config directories; normalise before comparing, since `Path` on Windows yields `\` |
| Long paths | Deep `imports/…/node_modules` trees can pass 260 characters: document `core.longpaths=true` and the Windows long-path setting |
| Line endings | `.gitscale.toml` read through `toml` is fine with CRLF; check `git show` output parsing in `src/stores.rs` and `git status --porcelain` parsing |

## Testing and CI

- Gate Unix-only test fixtures (`symlink`, `#!/bin/sh`) with `cfg(unix)`, and
  add Windows equivalents where the behaviour matters: links, readonly, hooks.
- Add a `windows-latest` job to `.github/workflows/ci.yml` running the full
  suite, plus one run with Developer Mode off to exercise the junction
  fallback.
- The fake OCI registry and `file://` remotes work as they are.

## Phases

1. **Builds and runs, no links**: `cfg`-gate every Unix API, portable readonly,
   cache and credential paths, `post_sync` through `sh`/`cmd`. CI job on
   Windows with link tests skipped. Workspaces without recursive dependencies
   work fully.
2. **Links**: `symlink_dir` with the junction fallback, one link-recognition
   helper used everywhere, relinking moved workspaces, safe removal.
3. **Hooks**: verify and document `hook install` on Git for Windows.

Rough size: phase 1 about a day, phase 2 a day or two, phase 3 half a day.

## Open questions

- Require Developer Mode instead of supporting junctions? Simpler, and it
  keeps relative links, at the cost of a setup step on every machine.
- Which `sh` should `post_sync` use when Git for Windows is not on `PATH`?
- Is a Windows runner cost acceptable for every CI run, or only nightly?

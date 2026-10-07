//! `gitscale hook` — install gitscale as git's hooks, so the workspace is
//! placed whenever a checkout or merge changes what is in a working tree —
//! the root's, or a child's — and every other hook a repository relies on
//! keeps running.
//!
//! A global or system install points `core.hooksPath` at a directory of its
//! own, and git then looks for every hook there and nowhere else. So besides
//! `post-checkout` and `post-merge`, which place the workspace, an install
//! writes a shim for each hook it is asked for with `--hooks`, and each hands
//! over to `gitscale hook run`, which runs, in order: the repository's own hook
//! in `.git/hooks`, git-lfs, the repository's committed `.githooks/`
//! (allowlisted), and the placement. The shim decides nothing itself, so a
//! newer gitscale changes what a hook does without a reinstall.
//!
//! `post-checkout` and `post-merge` between them cover clone,
//! checkout/switch, CI's `fetch --depth=1` + `checkout FETCH_HEAD`, merge,
//! `git pull` and rebase (which fires `post-checkout` too). `git reset --hard`
//! fires no worktree hook at all and is therefore not covered — `git scale ls`
//! remains the safety net there.

use anyhow::{bail, Context, Result};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{OnHookError, CONFIG_FILENAME};
use crate::trust;

/// The git hooks gitscale can install: every client-side hook git runs, but
/// `reference-transaction` and `post-index-change`, which fire on every ref
/// update and index write — a program started for each would be felt — and
/// `push-to-checkout`, `fsmonitor-watchman` and the server-side hooks, which
/// mean nothing on a developer's machine.
pub const HOOKS: [&str; 19] = [
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "post-rewrite",
    "pre-auto-gc",
    "sendemail-validate",
    "p4-changelist",
    "p4-prepare-changelist",
    "p4-post-changelist",
    "p4-pre-submit",
];

/// The hooks that place the workspace, which every install writes.
const PLACING: [&str; 2] = ["post-checkout", "post-merge"];

/// The hooks git-lfs installs, each of which is `git lfs <hook> "$@"`.
const LFS_HOOKS: [&str; 4] = ["pre-push", "post-checkout", "post-commit", "post-merge"];

/// The hooks git hands a payload on stdin. Every stage of one gets its own
/// copy: the first to read it would otherwise leave the rest nothing.
const STDIN_HOOKS: [&str; 2] = ["pre-push", "post-rewrite"];

/// Where a repository commits hooks for everyone who clones it.
pub const COMMITTED_HOOKS_DIR: &str = ".githooks";

/// Whether a failure of `hook` stops the operation git is running. The rest
/// run after the fact: every stage still runs, and the first failure is
/// reported.
fn can_abort(hook: &str) -> bool {
    !(hook.starts_with("post-") || hook == "p4-post-changelist")
}

/// Where an install writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// A hook file in this repository only.
    Local,
    /// `core.hooksPath` in the current user's ~/.gitconfig.
    Global,
    /// `core.hooksPath` in /etc/gitconfig, covering every user on the machine.
    System,
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Scope::Local => "local",
            Scope::Global => "global",
            Scope::System => "system",
        }
    }

    fn config_flag(self) -> &'static str {
        match self {
            Scope::Local => "--local",
            Scope::Global => "--global",
            Scope::System => "--system",
        }
    }
}

const SHIM_MARKER: &str = "# installed by gitscale";

/// What a shim passes that one written before `hook run` decided everything
/// does not: how `hook status` tells the two apart.
const SHIM_HANDOFF: &str = " --shim ";

/// Body of the installed hook. It hands everything to `gitscale hook run`,
/// so what a hook does is the installed gitscale's to decide, and an upgrade
/// needs no reinstall.
///
/// The binary, the scope and the allowlist are baked in at install time.
/// Baking the allowlist in is what makes it trustworthy: the shim sits in the
/// user's config directory or in /etc, so no branch can reach it, and a
/// repository therefore cannot say anything about whether its own committed
/// hooks may run.
///
/// Without the binary, the shim still runs the repository's own hook — the
/// one git would have run had `core.hooksPath` not pointed here — and says on
/// every hook that gitscale is gone, since every repository's other hooks
/// stopped with it.
fn shim_source(hook: &str, binary: &Path, scope: Scope, allow: &str) -> String {
    format!(
        r#"#!/bin/sh
{marker} — regenerate with `gitscale hook install`, do not edit.
#
# What this hook does — the repository's own hook, git-lfs, .githooks/, and
# placing a gitscale workspace — is decided by `gitscale hook run`.
GITSCALE_BIN={binary}
HOOK={hook}

# Which repositories may run what they commit themselves: hooks in .githooks/
# and [hooks] commands in .gitscale.toml. Comma-separated glob patterns, matched
# against host/owner/repo. Change it with `gitscale hook install --allow ...`,
# never by editing this line.
ALLOW={allow}

if [ ! -x "$GITSCALE_BIN" ]; then
    echo "gitscale: $GITSCALE_BIN not found; only this repository's own $HOOK hook runs. Reinstall with \`gitscale hook install\`, or remove with \`gitscale hook uninstall\`." >&2
    for OWN in "$0.local" "$(git rev-parse --git-common-dir 2>/dev/null)/hooks/$HOOK"; do
        if [ -f "$OWN" ] && [ -x "$OWN" ] && ! grep -q '{marker}' "$OWN"; then
            exec "$OWN" "$@"
        fi
    done
    exit 0
fi

{allow_env}="$ALLOW" exec "$GITSCALE_BIN" hook run "$HOOK"{handoff}"$0" --scope {scope} -- "$@"
"#,
        marker = SHIM_MARKER,
        binary = sh_quote(&binary.display().to_string()),
        hook = sh_quote(hook),
        allow = sh_quote(allow),
        allow_env = trust::ALLOW_ENV,
        handoff = SHIM_HANDOFF,
        scope = scope.label(),
    )
}

// ---------------------------------------------------------------------------
// Git config helpers
// ---------------------------------------------------------------------------

fn git_config_get(scope: Option<Scope>, key: &str, cwd: Option<&Path>) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("config");
    if let Some(s) = scope {
        cmd.arg(s.config_flag());
    }
    cmd.args(["--get", key]);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn git_stdout(args: &[&str], cwd: &Path) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to run: git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Where a `--local` install writes, and whether the repository chose it. A
/// repo-local `core.hooksPath` (husky, lefthook, pre-commit) overrides a global
/// one, so a global install is invisible in such repos — hence the flag.
///
/// Deliberately blind to a global or system `core.hooksPath`: `--local` means
/// this repository, and must not end up writing into the shared hooks directory
/// just because one is configured.
fn local_hooks_dir(repo: &Path) -> Result<(PathBuf, bool)> {
    if let Some(local) = git_config_get(Some(Scope::Local), "core.hooksPath", Some(repo)) {
        return Ok((resolve_hooks_path(repo, &local), true));
    }
    Ok((default_hooks_dir(repo)?, false))
}

/// The directory git will actually run this repo's hooks from — asked of git
/// unscoped, so a global or system `core.hooksPath` counts. This is what
/// `status` must report: anything else would say "not installed" about a hook
/// that is demonstrably running.
fn active_hooks_dir(repo: &Path) -> Result<(PathBuf, bool)> {
    let repo_chose_it = git_config_get(Some(Scope::Local), "core.hooksPath", Some(repo)).is_some();
    match git_config_get(None, "core.hooksPath", Some(repo)) {
        Some(path) => Ok((resolve_hooks_path(repo, &path), repo_chose_it)),
        None => Ok((default_hooks_dir(repo)?, repo_chose_it)),
    }
}

/// Where git looks for hooks when `core.hooksPath` is unset: the common git
/// directory's `hooks`, shared by every worktree. Not `--absolute-git-dir`,
/// which in a linked worktree is `.git/worktrees/<name>` — git never runs
/// hooks from there.
fn default_hooks_dir(repo: &Path) -> Result<PathBuf> {
    let common = git_stdout(&["rev-parse", "--git-common-dir"], repo)?;
    // Relative to `repo` in the main worktree, absolute in a linked one.
    let common = repo.join(common);
    Ok(common.canonicalize().unwrap_or(common).join("hooks"))
}

/// A relative `core.hooksPath` is resolved against the worktree root.
fn resolve_hooks_path(repo: &Path, configured: &str) -> PathBuf {
    let path = PathBuf::from(configured);
    if path.is_absolute() {
        path
    } else {
        repo.join(path)
    }
}

fn worktree_root(start: Option<&Path>) -> Result<PathBuf> {
    let cwd = match start {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().context("cannot get current directory")?,
    };
    let root = git_stdout(&["rev-parse", "--show-toplevel"], &cwd)
        .context("not inside a git repository")?;
    Ok(PathBuf::from(root))
}

fn gitscale_binary() -> Result<PathBuf> {
    std::env::current_exe()
        .context("cannot determine the path of the running gitscale binary")?
        .canonicalize()
        .context("cannot resolve the gitscale binary path")
}

fn is_shim(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|s| has_marker(&s))
        .unwrap_or(false)
}

/// The marker as the line a shim starts with — not merely the text, which the
/// shim's own fallback also holds, and which a repository's copy of a shim,
/// its header replaced, therefore still carries.
fn has_marker(text: &str) -> bool {
    text.lines().any(|line| line.starts_with(SHIM_MARKER))
}

/// A hook that hands off to `gitscale hook run`, whoever wrote it: a shim of
/// ours, or a copy a repository commits — a `.githooks/` directory its own
/// setup points `core.hooksPath` at — which carries no marker because gitscale
/// did not write it.
///
/// For reporting only. Install and uninstall act on `is_shim` alone, so a
/// committed copy is never displaced, rewritten or deleted.
fn runs_gitscale(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|s| {
            s.lines().any(|line| {
                !line.trim_start().starts_with('#')
                    && line.to_lowercase().contains("gitscale")
                    && line.contains(" hook run ")
            })
        })
        .unwrap_or(false)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | 0o755);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// install
// ---------------------------------------------------------------------------

pub fn install(
    scope: Scope,
    root: Option<&Path>,
    force: bool,
    allow: Option<&str>,
    hooks: Option<&str>,
    out: &mut dyn Write,
) -> Result<()> {
    let binary = gitscale_binary()?;

    // Validate before writing anything: a refused install must not leave hook
    // files scattered in a directory git was never pointed at.
    if scope != Scope::Local {
        if let Some(existing) = git_config_get(Some(scope), "core.hooksPath", None) {
            let intended = managed_hooks_dir(scope)?;
            if Path::new(&existing) != intended && !force {
                bail!(
                    "{} core.hooksPath is already set to {} — refusing to replace it (use --force).\n\
                     Installing over it would silently disable those hooks.",
                    scope.label(),
                    existing
                );
            }
        }
    }

    let dir = match scope {
        Scope::Local => {
            let repo = worktree_root(root)?;
            let (dir, repo_chose_it) = local_hooks_dir(&repo)?;
            if repo_chose_it {
                writeln!(
                    out,
                    "This repo sets its own core.hooksPath; installing into {}",
                    dir.display()
                )?;
            }
            dir
        }
        Scope::Global | Scope::System => managed_hooks_dir(scope)?,
    };

    let allow = resolve_allow(scope, &dir, allow)?;
    let chosen = resolve_hooks(&dir, hooks)?;

    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;

    // Every displacement is checked before any is made, so a refusal leaves
    // the directory as it was.
    if !force {
        for &hook in &chosen {
            let (target, displaced) = (dir.join(hook), displaced_path(&dir, hook));
            if target.exists() && !is_shim(&target) && displaced.exists() {
                bail!(
                    "{} already exists; refusing to overwrite it (use --force)",
                    displaced.display()
                );
            }
        }
    }

    for &hook in &chosen {
        let target = dir.join(hook);
        if target.exists() && !is_shim(&target) {
            // Somebody else's hook already lives here. Displace rather than
            // destroy it; `hook run` finds it beside the shim and runs it.
            let displaced = displaced_path(&dir, hook);
            std::fs::rename(&target, &displaced).with_context(|| {
                format!(
                    "cannot move {} aside to {}",
                    target.display(),
                    displaced.display()
                )
            })?;
            writeln!(
                out,
                "  moved existing {} to {}",
                hook,
                displaced.file_name().unwrap().to_string_lossy()
            )?;
        }

        std::fs::write(&target, shim_source(hook, &binary, scope, &allow))
            .with_context(|| format!("cannot write {}", target.display()))?;
        make_executable(&target)?;
    }
    writeln!(
        out,
        "  installed {} in {}",
        describe_hooks(&chosen),
        dir.display()
    )?;
    // A hook left out of the selection is one gitscale no longer handles.
    for hook in HOOKS.into_iter().filter(|h| !chosen.contains(h)) {
        if remove_shim(&dir, hook, out)? {
            writeln!(out, "  removed {}", hook)?;
        }
    }

    if scope != Scope::Local {
        set_hooks_path(scope, &dir, out)?;
    }

    writeln!(out, "  allowing {}", allow)?;
    let lfs_chosen = LFS_HOOKS.iter().all(|h| chosen.contains(h));
    if scope != Scope::Local && chosen.len() < HOOKS.len() {
        // git looks for every hook in this directory and nowhere else.
        writeln!(
            out,
            "  no other git hook runs in any repository, its own .git/hooks included; add \
             them with --hooks"
        )?;
        if lfs_binary().is_some() && !lfs_chosen {
            writeln!(
                out,
                "  git-lfs is installed, but its hooks are not: add --hooks lfs, or a push \
                 leaves the large files behind"
            )?;
        }
    }
    // git-lfs writes its own hooks wherever `core.hooksPath` points, and
    // refuses to replace ours; `hook run` already runs it.
    if lfs_binary().is_some() && lfs_chosen {
        writeln!(
            out,
            "  git-lfs runs through these hooks; set it up with `git lfs install --skip-repo`"
        )?;
    }
    // `git <cmd> --help` is a man page lookup: the pages go beside the
    // binary's own share directory, where `man` finds them.
    if let Some(dir) = crate::man::write() {
        writeln!(out, "  man pages in {}", dir.display())?;
    }
    writeln!(out, "gitscale hooks installed ({})", scope.label())?;
    Ok(())
}

/// Decide the allowlist to bake in.
///
/// A `--global` or `--system` install arms every `git clone` on the machine, so
/// it has to be told what to trust — there is no sensible default, and guessing
/// one is the whole bug. A `--local` install is already a statement about one
/// repository, and its hook file cannot travel to anyone else, so it allows that
/// repository. Re-installing keeps whatever the previous shim allowed, so an
/// upgrade does not silently widen or narrow anything.
fn resolve_allow(scope: Scope, dir: &Path, requested: Option<&str>) -> Result<String> {
    if let Some(spec) = requested {
        trust::validate_spec(spec)?;
        return Ok(spec.to_string());
    }
    if let Some(existing) = existing_allow(dir) {
        return Ok(existing);
    }
    if scope == Scope::Local {
        return Ok(trust::ALLOW_ANY.to_string());
    }
    bail!(
        "--allow is required for a {} install.\n\n\
         A {} hook runs on every clone and checkout on this machine, including of a \
         branch\nyou are only reviewing — and a branch can carry hooks of its own, in \
         .githooks/ or as\n[hooks] commands in .gitscale.toml. --allow decides which \
         repositories those may come\nfrom, as comma-separated glob patterns matched against \
         host/owner/repo:\n\n\
         \x20   gitscale hook install --{} --allow 'github.com/acme/*,git.internal.example/*'\n\n\
         Use --allow '{}' to allow every repository.",
        scope.label(),
        scope.label(),
        scope.label(),
        trust::ALLOW_ANY
    )
}

/// Decide the hooks to install, in [`HOOKS`] order: the placing hooks, and
/// what `--hooks` names — hook names, `lfs` for git-lfs's four, `all` for
/// every one. Re-installing without `--hooks` keeps the hooks installed
/// before, so an upgrade neither adds nor drops one.
fn resolve_hooks(dir: &Path, requested: Option<&str>) -> Result<Vec<&'static str>> {
    let mut wanted: Vec<&str> = PLACING.to_vec();
    match requested {
        None => wanted.extend(HOOKS.iter().filter(|h| is_shim(&dir.join(h)))),
        Some(spec) => {
            for word in spec.split(',').map(str::trim).filter(|w| !w.is_empty()) {
                match word {
                    "all" => wanted.extend(HOOKS),
                    "lfs" => wanted.extend(LFS_HOOKS),
                    name => match HOOKS.iter().find(|h| **h == name) {
                        Some(hook) => wanted.push(hook),
                        None => bail!(
                            "unknown hook '{}' in --hooks. Name any of:\n  {}\nor `lfs` for \
                             git-lfs's hooks, or `all`.",
                            name,
                            HOOKS.join(", ")
                        ),
                    },
                }
            }
        }
    }
    Ok(HOOKS.into_iter().filter(|h| wanted.contains(h)).collect())
}

/// The hooks `hooks` names, for one line of output.
fn describe_hooks(hooks: &[&str]) -> String {
    if hooks.len() == HOOKS.len() {
        format!("all {} hooks", hooks.len())
    } else {
        hooks.join(", ")
    }
}

/// Take a shim of ours out of `dir`, putting back the hook it displaced.
/// Whether there was one to take.
fn remove_shim(dir: &Path, hook: &str, out: &mut dyn Write) -> Result<bool> {
    let target = dir.join(hook);
    if !is_shim(&target) {
        return Ok(false);
    }
    std::fs::remove_file(&target).with_context(|| format!("cannot remove {}", target.display()))?;
    // Put the repo's own hook back where git expects it.
    let displaced = displaced_path(dir, hook);
    if displaced.exists() {
        std::fs::rename(&displaced, &target)
            .with_context(|| format!("cannot restore {}", target.display()))?;
        writeln!(out, "  restored {}", target.display())?;
    }
    Ok(true)
}

/// Read the allowlist out of a shim we previously wrote.
fn existing_allow(dir: &Path) -> Option<String> {
    HOOKS.iter().find_map(|hook| {
        let target = dir.join(hook);
        is_shim(&target)
            .then(|| shim_field(&target, "ALLOW"))
            .flatten()
    })
}

/// Pull a single-quoted assignment back out of a shim.
fn shim_field(shim: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(shim).ok()?;
    let prefix = format!("{}=", name);
    let line = text.lines().find(|l| l.starts_with(&prefix))?;
    sh_unquote(&line[prefix.len()..])
}

/// `value` as one single-quoted shell word. A path may hold a `'`, and
/// pasted into the shim as it is, the rest of it would run as shell code.
fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// The value of a shell word `sh_quote` wrote: quoted runs and `\'` escapes.
fn sh_unquote(word: &str) -> Option<String> {
    let mut value = String::new();
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => loop {
                match chars.next()? {
                    '\'' => break,
                    c => value.push(c),
                }
            },
            '\\' => value.push(chars.next()?),
            _ => return None,
        }
    }
    Some(value)
}

/// Where install moves a hook it finds in its way, beside the shim.
fn displaced_path(dir: &Path, hook: &str) -> PathBuf {
    dir.join(format!("{}.local", hook))
}

/// Where global/system installs keep their hooks. Kept beside the git config
/// they are referenced from so an uninstall can reason about ownership.
fn managed_hooks_dir(scope: Scope) -> Result<PathBuf> {
    let base = match scope {
        Scope::System => PathBuf::from("/etc/gitscale"),
        _ => {
            let home = std::env::var("HOME").context("HOME is not set")?;
            PathBuf::from(home).join(".config/gitscale")
        }
    };
    Ok(base.join("hooks"))
}

fn set_hooks_path(scope: Scope, dir: &Path, out: &mut dyn Write) -> Result<()> {
    let status = Command::new("git")
        .args(["config", scope.config_flag(), "core.hooksPath"])
        .arg(dir)
        .status()
        .context("failed to run git config")?;
    if !status.success() {
        bail!("could not set {} core.hooksPath", scope.label());
    }
    writeln!(
        out,
        "  set {} core.hooksPath to {}",
        scope.label(),
        dir.display()
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// uninstall
// ---------------------------------------------------------------------------

pub fn uninstall(scope: Scope, root: Option<&Path>, out: &mut dyn Write) -> Result<()> {
    let dir = match scope {
        Scope::Local => {
            let repo = worktree_root(root)?;
            local_hooks_dir(&repo)?.0
        }
        Scope::Global | Scope::System => managed_hooks_dir(scope)?,
    };

    let mut removed = 0;
    for hook in HOOKS {
        if remove_shim(&dir, hook, out)? {
            removed += 1;
        }
    }
    if removed > 0 {
        writeln!(out, "  removed {} hooks from {}", removed, dir.display())?;
    }

    if scope != Scope::Local {
        let current = git_config_get(Some(scope), "core.hooksPath", None);
        if current.as_deref() == Some(dir.to_string_lossy().as_ref()) {
            let _ = Command::new("git")
                .args(["config", scope.config_flag(), "--unset", "core.hooksPath"])
                .status();
            writeln!(out, "  unset {} core.hooksPath", scope.label())?;
        }
    }

    if removed == 0 {
        writeln!(out, "No gitscale hooks were installed ({}).", scope.label())?;
    } else {
        writeln!(out, "gitscale hooks removed ({}).", scope.label())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

pub fn status(root: Option<&Path>, out: &mut dyn Write) -> Result<()> {
    for scope in [Scope::System, Scope::Global] {
        match git_config_get(Some(scope), "core.hooksPath", None) {
            Some(path) => writeln!(out, "{:<8} core.hooksPath = {}", scope.label(), path)?,
            None => writeln!(out, "{:<8} core.hooksPath unset", scope.label())?,
        }
    }

    let Ok(repo) = worktree_root(root) else {
        writeln!(out, "\nNot inside a git repository.")?;
        return Ok(());
    };
    let (dir, repo_chose_it) = active_hooks_dir(&repo)?;
    writeln!(out, "\nrepo     {}", repo.display())?;
    writeln!(out, "hooks    {}", dir.display())?;

    if repo_chose_it {
        let shadowed = git_config_get(Some(Scope::Global), "core.hooksPath", None).is_some()
            || git_config_get(Some(Scope::System), "core.hooksPath", None).is_some();
        if shadowed {
            // The advice depends on what is in the directory the repo chose: a
            // monorepo that commits its own copies of the shim is covered, and
            // telling it to install over them would be wrong.
            let covered = PLACING.iter().all(|hook| runs_gitscale(&dir.join(hook)));
            if covered {
                writeln!(
                    out,
                    "\nThis repo sets its own core.hooksPath, which overrides the global one.\n\
                     Its own hooks there run gitscale, so nothing is lost."
                )?;
            } else {
                writeln!(
                    out,
                    "\nThis repo sets its own core.hooksPath, which overrides the global one.\n\
                     A global gitscale install does not run here — use `gitscale hook install --local`."
                )?;
            }
        }
    }

    // One line per state, naming the hooks in it: nineteen lines of
    // "gitscale" would bury the one that is not.
    const OUTDATED: &str = "gitscale, written by an older version";
    let mut states: Vec<(&str, Vec<&str>)> = Vec::new();
    for hook in HOOKS {
        let target = dir.join(hook);
        let state = if !target.exists() {
            "not installed"
        } else if is_shim(&target) {
            let current =
                std::fs::read_to_string(&target).is_ok_and(|text| text.contains(SHIM_HANDOFF));
            if current {
                "gitscale"
            } else {
                OUTDATED
            }
        } else if runs_gitscale(&target) {
            "gitscale (repository's own copy)"
        } else {
            "other (not gitscale)"
        };
        match states.iter_mut().find(|(s, _)| *s == state) {
            Some((_, hooks)) => hooks.push(hook),
            None => states.push((state, vec![hook])),
        }
    }
    for (state, hooks) in &states {
        writeln!(out, "  {:<38} {}", state, describe_hooks(hooks))?;
    }
    if states.iter().any(|(s, _)| *s == OUTDATED) {
        writeln!(
            out,
            "\nSome hooks were written by an older gitscale, which runs neither this repository's\n\
             other hooks nor git-lfs. Re-run `gitscale hook install` with the scope it was\n\
             installed at."
        )?;
    }
    // Under a hooks path the repository did not choose, git runs no hook
    // from its own .git/hooks: one not installed here runs nowhere.
    let shared = !repo_chose_it && dir != default_hooks_dir(&repo)?;
    if shared && states.iter().any(|(s, _)| *s == "not installed") {
        writeln!(
            out,
            "\nHooks not installed here do not run in this repository at all, not even from its own\n\
             .git/hooks. Add them with `gitscale hook install --hooks ...`."
        )?;
    }

    report_trust(&repo, &dir, out)?;

    if let Some(note) = read_breadcrumb(&repo)? {
        writeln!(
            out,
            "\nLast hook-triggered placement FAILED:\n  {}",
            note.trim()
        )?;
    }
    Ok(())
}

/// What the installed hook allows, and whether this repository is inside it —
/// the question people have once a hook has been refused.
fn report_trust(repo: &Path, dir: &Path, out: &mut dyn Write) -> Result<()> {
    // A repository's own copy passes its allowlist the same way a shim does.
    let spec = existing_allow(dir).or_else(|| {
        HOOKS.iter().find_map(|hook| {
            let target = dir.join(hook);
            runs_gitscale(&target)
                .then(|| shim_field(&target, "ALLOW"))
                .flatten()
        })
    });
    let Some(spec) = spec else {
        return Ok(());
    };
    let allowlist = trust::Allowlist::parse(&spec);

    writeln!(out, "\nhook allowlist")?;
    if allowlist.patterns().is_empty() {
        writeln!(
            out,
            "  (empty — no repository may run its .githooks/ or [hooks] commands)"
        )?;
    }
    for pattern in allowlist.patterns() {
        writeln!(out, "  {}", pattern)?;
    }

    let workspace = trust::Workspace::probe(repo);
    match allowlist.matched_by(&workspace) {
        Some(pattern) => writeln!(
            out,
            "\nthis repo  {} — allowed by '{}'",
            workspace.describe(),
            pattern
        )?,
        None => writeln!(
            out,
            "\nthis repo  {} — NOT allowed; its .githooks/ and [hooks] commands would be refused",
            workspace.describe()
        )?,
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// run — invoked by the installed shim
// ---------------------------------------------------------------------------

fn breadcrumb_path(repo: &Path) -> Result<PathBuf> {
    let git_dir = git_stdout(&["rev-parse", "--absolute-git-dir"], repo)?;
    Ok(PathBuf::from(git_dir).join("gitscale-pull-failed"))
}

fn read_breadcrumb(repo: &Path) -> Result<Option<String>> {
    let path = breadcrumb_path(repo)?;
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(Some(s)),
        Err(_) => Ok(None),
    }
}

/// The process exits with this status and prints nothing more: a hook that
/// failed has said why itself.
#[derive(Debug)]
pub struct HookExit(pub i32);

impl std::fmt::Display for HookExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a hook exited with status {}", self.0)
    }
}

impl std::error::Error for HookExit {}

/// What an installed shim hands to `hook run`: its own path, the scope it was
/// installed at, and git's arguments.
pub struct Handoff<'a> {
    pub shim: &'a Path,
    pub scope: Scope,
    pub args: &'a [OsString],
}

impl Scope {
    /// The scope a shim names with `--scope`.
    pub fn parse(label: &str) -> Result<Self> {
        match label {
            "local" => Ok(Scope::Local),
            "global" => Ok(Scope::Global),
            "system" => Ok(Scope::System),
            other => bail!("unknown hook scope '{}' (local, global or system)", other),
        }
    }
}

/// Run a git hook, as the installed shim asks.
///
/// With a [`Handoff`] — every shim this version writes — the stages of the
/// hook run in turn: the repository's own hook, git-lfs, `.githooks/`, and the
/// placement. Without one, the caller is a shim written before that, which
/// fired for `post-checkout` and `post-merge` only, ran the repository's hook
/// itself and passes none of git's arguments: it gets the placement alone.
#[allow(clippy::too_many_arguments)]
pub fn run(
    hook: &str,
    root: Option<&Path>,
    child: Option<&Path>,
    handoff: Option<Handoff>,
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let known: &[&str] = if handoff.is_some() { &HOOKS } else { &PLACING };
    if !known.contains(&hook) {
        bail!(
            "unknown hook '{}' (gitscale installs: {})",
            hook,
            known.join(", ")
        );
    }

    // Every shim exports this, so its absence means one of two things: a shim
    // written by a gitscale that predates the allowlist, or someone running the
    // subcommand by hand. Both are refused rather than run wide open — an
    // upgrade must not leave the old behaviour quietly in place.
    let Some(allowlist) = trust::Allowlist::from_env() else {
        bail!(
            "`gitscale hook run` is invoked by the installed hook, which passes {} to it.\n\
             That variable is not set, so this hook was installed by an older gitscale.\n\
             Reinstall it to choose what it may run:\n\n    \
             gitscale hook install --global --allow 'github.com/acme/*'",
            trust::ALLOW_ENV
        );
    };

    match handoff {
        Some(handoff) => dispatch(
            hook,
            &handoff,
            &allowlist,
            verbose,
            no_cache,
            interactive,
            out,
            err,
        ),
        None => place(hook, root, child, verbose, no_cache, interactive, out, err),
    }
}

/// Where git ran a hook. Found without running git, since every hook in every
/// repository on the machine pays for it: git starts a hook at the top of the
/// worktree, or in the git directory of a bare repository, and sometimes
/// exports `GIT_DIR` — absolute in a linked worktree, relative in a clone.
struct Site {
    /// The git directory every worktree shares: where `hooks/` and `lfs/` live.
    common: PathBuf,
    /// The top of the worktree; `None` in a bare repository.
    top: Option<PathBuf>,
}

impl Site {
    fn here() -> Result<Self> {
        let cwd = std::env::current_dir().context("cannot get current directory")?;
        let git_dir = match std::env::var_os("GIT_DIR").filter(|d| !d.is_empty()) {
            Some(dir) => cwd.join(dir),
            None => {
                let dot = cwd.join(".git");
                if dot.is_dir() {
                    dot
                } else if dot.is_file() {
                    let text = std::fs::read_to_string(&dot)
                        .with_context(|| format!("cannot read {}", dot.display()))?;
                    let target = text
                        .lines()
                        .find_map(|l| l.strip_prefix("gitdir:"))
                        .with_context(|| format!("{} names no gitdir", dot.display()))?;
                    cwd.join(target.trim())
                } else {
                    cwd.clone()
                }
            }
        };
        let common = match std::fs::read_to_string(git_dir.join("commondir")) {
            Ok(rel) => git_dir.join(rel.trim()),
            Err(_) => git_dir.clone(),
        };
        let canonical = |p: PathBuf| p.canonicalize().unwrap_or(p);
        let (git_dir, cwd) = (canonical(git_dir), canonical(cwd));
        let top = match std::env::var_os("GIT_WORK_TREE").filter(|d| !d.is_empty()) {
            Some(tree) => Some(canonical(cwd.join(tree))),
            None => (git_dir != cwd).then_some(cwd),
        };
        Ok(Site {
            common: canonical(common),
            top,
        })
    }
}

/// What a hook file is, to the dispatcher.
#[derive(PartialEq)]
enum Kind {
    /// It runs `gitscale hook run` — a shim, or a repository's copy of one.
    /// Running it from here would run everything a second time.
    Gitscale,
    /// The hook git-lfs installs, nothing added: the git-lfs stage covers it.
    Lfs,
    Other,
}

fn kind(path: &Path, hook: &str) -> Kind {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Kind::Other;
    };
    if has_marker(&text) || runs_gitscale(path) {
        return Kind::Gitscale;
    }
    let call = format!("git lfs {} \"$@\"", hook);
    let mut calls_lfs = false;
    let only_lfs = text.lines().map(str::trim).all(|line| {
        if line == call {
            calls_lfs = true;
        }
        line.is_empty()
            || line.starts_with('#')
            || line.starts_with("command -v git-lfs")
            || line == call
    });
    if only_lfs && calls_lfs {
        Kind::Lfs
    } else {
        Kind::Other
    }
}

/// A file git would run as a hook: present and executable.
fn runnable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// The git-lfs executable, when one is installed where git would find it.
fn lfs_binary() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "git-lfs.exe"
    } else {
        "git-lfs"
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(exec) = std::env::var_os("GIT_EXEC_PATH") {
        dirs.push(PathBuf::from(exec));
    }
    dirs.into_iter().map(|d| d.join(name)).find(|p| runnable(p))
}

/// Run one stage of a hook: its exit status, or 128 plus the signal that
/// ended it, as a shell would report it.
fn run_stage(mut cmd: Command, stdin: Option<&[u8]>) -> Result<i32> {
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::inherit()
    });
    let mut child = cmd
        .spawn()
        .with_context(|| format!("cannot run {:?}", cmd.get_program()))?;
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // A hook that never reads its payload closes the pipe on us: fine.
        let _ = pipe.write_all(bytes);
    }
    let status = child.wait()?;
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return Ok(128 + signal);
        }
    }
    Ok(status.code().unwrap_or(1))
}

/// Run a hook file with git's arguments. A script with no `#!` line is run
/// by `sh`, as git runs one.
fn run_file(path: &Path, args: &[OsString], stdin: Option<&[u8]>) -> Result<i32> {
    let mut cmd = Command::new(path);
    cmd.args(args);
    match run_stage(cmd, stdin) {
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .and_then(std::io::Error::raw_os_error)
                == Some(ENOEXEC) =>
        {
            let mut cmd = Command::new("sh");
            cmd.arg(path).args(args);
            run_stage(cmd, stdin)
        }
        other => other,
    }
}

/// `ENOEXEC`, the same on Linux and macOS: the file is executable, but not
/// in a format the kernel runs.
const ENOEXEC: i32 = 8;

/// Record a stage's exit status. A failure of a hook that can stop git's
/// operation stops the rest; any other is remembered, and the rest run.
fn settle(hook: &str, code: i32, failed: &mut Option<i32>) -> Result<()> {
    if code == 0 {
        return Ok(());
    }
    if can_abort(hook) {
        return Err(HookExit(code).into());
    }
    failed.get_or_insert(code);
    Ok(())
}

/// The stages of a hook, in order:
///
/// 1. the repository's own hook — the one beside the shim that install moved
///    aside, and, for a global or system install, `<common dir>/hooks/<name>`,
///    which git no longer looks at;
/// 2. git-lfs, for the hooks it installs, in a repository that uses it;
/// 3. `.githooks/<name>` at the top of the worktree, when the repository is on
///    the allowlist;
/// 4. the placement, for `post-checkout` and `post-merge`.
///
/// A hook file that runs gitscale is skipped wherever it is found, and so is
/// git-lfs's own hook: stage 2 runs git-lfs once.
#[allow(clippy::too_many_arguments)]
fn dispatch(
    hook: &str,
    handoff: &Handoff,
    allowlist: &trust::Allowlist,
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let site = Site::here()?;
    let args = handoff.args;
    let stdin = if STDIN_HOOKS.contains(&hook) {
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut std::io::stdin(), &mut bytes)
            .context("cannot read the hook's input")?;
        Some(bytes)
    } else {
        None
    };
    let stdin = stdin.as_deref();
    let mut failed = None;

    // 1. The repository's own hook.
    let shim = handoff
        .shim
        .canonicalize()
        .unwrap_or_else(|_| handoff.shim.to_path_buf());
    let shim_dir = shim.parent().unwrap_or(Path::new("/"));
    let mut own = vec![displaced_path(shim_dir, hook)];
    if handoff.scope != Scope::Local {
        let hooks = site.common.join("hooks");
        if hooks.canonicalize().ok().as_deref() != Some(shim_dir) {
            own.push(hooks.join(hook));
        }
    }
    let mut lfs_hook = false;
    for path in own.into_iter().filter(|p| runnable(p)) {
        match kind(&path, hook) {
            Kind::Gitscale => {}
            Kind::Lfs => lfs_hook = true,
            Kind::Other => settle(hook, run_file(&path, args, stdin)?, &mut failed)?,
        }
    }

    // 2. git-lfs. A repository uses it once git-lfs has a store in it, or a
    // hook git-lfs installed there says so.
    if LFS_HOOKS.contains(&hook) && (lfs_hook || site.common.join("lfs").is_dir()) {
        let code = if lfs_binary().is_some() {
            let mut cmd = Command::new("git");
            cmd.arg("lfs").arg(hook).args(args);
            run_stage(cmd, stdin)?
        } else {
            writeln!(
                err,
                "gitscale: this repository uses Git LFS, but git-lfs is not installed. Install \
                 it, or, if the repository no longer uses Git LFS, delete {}.",
                site.common.join("lfs").display()
            )?;
            2
        };
        settle(hook, code, &mut failed)?;
    }

    // 3. The repository's committed hook, if the allowlist lets it run.
    if let Some(top) = &site.top {
        let committed = top.join(COMMITTED_HOOKS_DIR).join(hook);
        if runnable(&committed) && kind(&committed, hook) != Kind::Gitscale {
            let workspace = trust::Workspace::probe(top);
            if allowlist.matched_by(&workspace).is_some() {
                settle(hook, run_file(&committed, args, stdin)?, &mut failed)?;
            } else {
                writeln!(
                    err,
                    "gitscale: {}/{} not run — {} is not on this machine's hook allowlist; see \
                     `gitscale hook status`",
                    COMMITTED_HOOKS_DIR,
                    hook,
                    trust::sanitize(&workspace.describe())
                )?;
            }
        }
    }

    // 4. The placement. Never when gitscale itself ran the git command —
    // every git call it makes is marked, and a placement running `git
    // checkout` would otherwise recurse without bound — and never for a file
    // checkout: post-checkout's third argument is 0 for `git checkout --
    // <path>`, which moves no revision, and a build restoring one file must
    // not have its sub-repositories reset and cleaned underneath it.
    let ours = std::env::var_os("GITSCALE_HOOK").is_some_and(|v| !v.is_empty());
    let file_checkout = hook == "post-checkout" && args.get(2).is_some_and(|a| a == "0");
    if PLACING.contains(&hook) && !ours && !file_checkout {
        if let Some(top) = &site.top {
            // A child — a checkout gitscale made, whose common dir is one of
            // a root's stores — places the workspace it belongs to. Anything
            // else opts in with a config at its own top: deliberately not the
            // usual upward search, or an unrelated repository cloned inside a
            // workspace would place the whole thing.
            let result = if crate::config::child_of(&site.common).is_some() {
                place(
                    hook,
                    None,
                    Some(top),
                    verbose,
                    no_cache,
                    interactive,
                    out,
                    err,
                )
            } else if top.join(CONFIG_FILENAME).is_file() {
                place(
                    hook,
                    Some(top),
                    None,
                    verbose,
                    no_cache,
                    interactive,
                    out,
                    err,
                )
            } else {
                Ok(())
            };
            if let Err(e) = result {
                if failed.is_none() {
                    return Err(e);
                }
                writeln!(err, "gitscale: {} hook — {:#}", hook, e)?;
            }
        }
    }

    match failed {
        Some(code) => Err(HookExit(code).into()),
        None => Ok(()),
    }
}

/// Place the workspace for a git hook.
///
/// A failure here is reported three ways, because none alone is sufficient: on
/// stderr (which reaches the user through `git clone`), in a breadcrumb file so
/// a later, more confusing failure can be traced back to this one, and — under
/// the `fail` policy — in the exit status. The exit status is the weakest of
/// the three: git collapses any non-zero hook exit to 1 and reports it as the
/// *checkout* failing, which it did not.
#[allow(clippy::too_many_arguments)]
fn place(
    hook: &str,
    root: Option<&Path>,
    child: Option<&Path>,
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (repo, leave) = match child {
        Some(child) => {
            let child = child
                .canonicalize()
                .with_context(|| format!("cannot resolve {}", child.display()))?;
            // In the middle of a rebase, a merge, a cherry-pick, a revert or a
            // bisect, the child is not where it will end: nothing moves yet.
            if mid_operation(&child) {
                return Ok(());
            }
            let root = crate::config::find_root(Some(&child))?;
            let leave = child
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().into_owned())
                .ok();
            (root, leave)
        }
        None => {
            let repo = worktree_root(root)?;
            // The shim checks this too; re-check so a hand-run `hook run` behaves.
            if !repo.join(CONFIG_FILENAME).is_file() {
                return Ok(());
            }
            (repo, None)
        }
    };

    let config = crate::config::load_config(&repo.join(CONFIG_FILENAME));
    let policy = {
        let configured = config.as_ref().ok().and_then(|c| c.hooks.on_pull_error);
        OnHookError::resolved(configured)
    };

    let result = config.and_then(|config| {
        crate::commands::sync::place(
            &config,
            &repo,
            &crate::commands::sync::Placement {
                dirs: &[],
                // A hook in the root follows a clone, a switch or a merge:
                // online. One in a child fetches only what it lacks.
                network: if leave.is_some() {
                    crate::resolve::Network::OnMiss
                } else {
                    crate::resolve::Network::Online
                },
                force: false,
                leave: leave.as_deref(),
                heading: "Pulling latest changes...",
            },
            verbose,
            no_cache,
            interactive,
            out,
            err,
        )?;
        if let Some(dir) = &leave {
            settle_child(&config, &repo, dir, no_cache, err)?;
        }
        Ok(())
    });

    let breadcrumb = breadcrumb_path(&repo)?;

    // Checked after the placement, when the checkouts exist for `git clean -n`
    // to find, and failed whatever `on_pull_error` says: that policy is for a
    // placement that did not work, and this is a pipeline that cannot — the
    // runner is about to delete what the placement just produced.
    if let Err(e) = crate::gitlab::check_runner_clean(&repo) {
        let note = format!("{}: {} hook: {}\n", now_stamp(), hook, e);
        let _ = std::fs::write(&breadcrumb, &note);
        writeln!(err, "gitscale: {} hook — {}", hook, e)?;
        // Told in full just above; the error only has to fail the checkout.
        bail!(
            "{} hook: GIT_CLEAN_FLAGS would delete the declared checkouts",
            hook
        );
    }

    match result {
        Ok(()) => {
            let _ = std::fs::remove_file(&breadcrumb);
            Ok(())
        }
        Err(e) => {
            let note = format!(
                "{}: the placement the {} hook ran failed: {}\n",
                now_stamp(),
                hook,
                e
            );
            let _ = std::fs::write(&breadcrumb, &note);
            writeln!(err, "gitscale: {} hook — placement failed: {}", hook, e)?;
            match policy {
                OnHookError::Fail => Err(e),
                OnHookError::Warn => {
                    writeln!(
                        err,
                        "gitscale: continuing anyway (hooks.on_pull_error = \"warn\"); \
                         see `gitscale hook status`"
                    )?;
                    Ok(())
                }
            }
        }
    }
}

/// Whether the repository at `dir` is in the middle of an operation git
/// will finish later.
fn mid_operation(dir: &Path) -> bool {
    [
        "rebase-merge",
        "rebase-apply",
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "BISECT_LOG",
    ]
    .iter()
    .any(|name| crate::git::git_path(dir, name).is_some_and(|p| p.exists()))
}

/// The child at `dir` after a placement that left it where git put it: on
/// its slot's topic branch it is made writable, as `git topic join` would;
/// on any other branch it stays, with a warning.
fn settle_child(
    config: &crate::config::GitScaleConfig,
    root: &Path,
    dir: &str,
    no_cache: bool,
    err: &mut dyn Write,
) -> Result<()> {
    let dest = root.join(dir);
    let Some(branch) = crate::git::current_branch(&dest) else {
        return Ok(());
    };
    let sources = crate::store::Sources::new(root, no_cache)?;
    let resolution = crate::resolve::workspace(config, root, false, &sources, None, false)?;
    let wanted = resolution.slot(dir).and_then(|s| s.branch.clone());
    if wanted.as_deref() == Some(branch.as_str()) {
        crate::git::restore_writable(&dest)?;
        return Ok(());
    }
    let color = crate::output::stderr();
    let message = match wanted {
        Some(topic) => format!(
            "{} is on {}, not the topic {}; the next placement moves it back to its pin",
            dir, branch, topic
        ),
        None => format!(
            "{} is on {}, and the workspace is on no topic; the next placement moves it back \
             to its pin",
            dir, branch
        ),
    };
    writeln!(
        err,
        "{}",
        crate::output::paint(color, crate::output::YELLOW, &message)
    )?;
    Ok(())
}

fn now_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quoted_value_reads_back_as_itself() {
        for value in [
            "",
            "/usr/bin/gitscale",
            "/home/o'brien/it's/hook",
            "'",
            "a b",
        ] {
            let word = sh_quote(value);
            assert_eq!(sh_unquote(&word).as_deref(), Some(value), "{}", word);
        }
    }

    #[test]
    fn a_shim_written_before_quoting_still_reads() {
        assert_eq!(sh_unquote("'/x/y'").as_deref(), Some("/x/y"));
        assert_eq!(sh_unquote("unquoted"), None);
    }
}

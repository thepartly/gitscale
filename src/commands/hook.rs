//! `gitscale hook` — install gitscale as a git hook so `gitscale pull` runs
//! whenever a checkout or merge changes what is in the working tree.
//!
//! Only `post-checkout` and `post-merge` are installed. Between them they
//! cover clone, checkout/switch, CI's `fetch --depth=1` + `checkout
//! FETCH_HEAD`, merge, `git pull` and rebase (which fires `post-checkout`
//! too). Neither reads stdin, so chaining to a repo's own hook cannot lose a
//! payload. `git reset --hard` fires no worktree hook at all and is therefore
//! not covered — `gitscale status` remains the safety net there.

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{load_config_optional, OnHookError, CONFIG_FILENAME};

/// The git hooks gitscale installs.
pub const HOOKS: [&str; 2] = ["post-checkout", "post-merge"];

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

/// Body of the installed hook. Both the gitscale binary and the chained hook
/// are baked in at install time: resolving them at run time would mean asking
/// git where hooks live, and `git rev-parse --git-path hooks/<name>` honours
/// `core.hooksPath` — which resolves right back to this shim.
fn shim_source(hook: &str, binary: &Path, chain: Option<&Path>) -> String {
    format!(
        r#"#!/bin/sh
{marker} — regenerate with `gitscale hook install`, do not edit.
set -u
GITSCALE_BIN='{binary}'
CHAIN='{chain}'
HOOK='{hook}'

# The repository's own hook runs first and decides the exit status. A global
# core.hooksPath replaces .git/hooks rather than adding to it, so without this
# every repo's own hooks would silently stop running.
RC=0
if [ -n "$CHAIN" ] && [ -x "$CHAIN" ]; then
    "$CHAIN" "$@" || RC=$?
fi

# Every git call gitscale makes sets this. `gitscale pull` runs `git checkout`,
# which fires this hook again; without the guard that recurses without bound.
if [ -n "${{GITSCALE_HOOK:-}}" ]; then
    exit $RC
fi

# Opt-in check, at the worktree root only. Deliberately not gitscale's usual
# upward search: an unrelated repo cloned inside a gitscale workspace would
# otherwise inherit the parent config and trigger a pull of the whole thing.
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit $RC
[ -f "$ROOT/{config}" ] || exit $RC

# Checked only after we know this repo opted in, so repos that never asked for
# gitscale stay completely silent while a broken install still gets reported.
if [ ! -x "$GITSCALE_BIN" ]; then
    echo "gitscale: $HOOK skipped — $GITSCALE_BIN not found, but $ROOT has {config}" >&2
    exit $RC
fi

GITSCALE_HOOK="$HOOK" "$GITSCALE_BIN" hook run "$HOOK" -C "$ROOT" || RC=$?
exit $RC
"#,
        marker = SHIM_MARKER,
        binary = binary.display(),
        chain = chain.map(|p| p.display().to_string()).unwrap_or_default(),
        hook = hook,
        config = CONFIG_FILENAME,
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

/// The directory git will actually look in for this repo's hooks, and whether
/// that location was chosen by the repository itself. A repo-local
/// `core.hooksPath` (husky, lefthook, pre-commit) overrides a global one, so a
/// global install is invisible in such repos — hence the flag.
fn effective_hooks_dir(repo: &Path) -> Result<(PathBuf, bool)> {
    if let Some(local) = git_config_get(Some(Scope::Local), "core.hooksPath", Some(repo)) {
        // A relative hooksPath is resolved against the worktree root.
        let path = PathBuf::from(&local);
        let abs = if path.is_absolute() {
            path
        } else {
            repo.join(path)
        };
        return Ok((abs, true));
    }
    let git_dir = git_stdout(&["rev-parse", "--absolute-git-dir"], repo)?;
    Ok((PathBuf::from(git_dir).join("hooks"), false))
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
        .map(|s| s.contains(SHIM_MARKER))
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

pub fn install(scope: Scope, root: Option<&Path>, force: bool, out: &mut dyn Write) -> Result<()> {
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
            let (dir, repo_chose_it) = effective_hooks_dir(&repo)?;
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

    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;

    for hook in HOOKS {
        let target = dir.join(hook);
        let mut chain: Option<PathBuf> = None;

        if target.exists() && !is_shim(&target) {
            // Somebody else's hook already lives here. Displace rather than
            // destroy it, and have the shim call it.
            let displaced = dir.join(format!("{}.local", hook));
            if displaced.exists() && !force {
                bail!(
                    "{} already exists; refusing to overwrite it (use --force)",
                    displaced.display()
                );
            }
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
            chain = Some(displaced);
        } else if target.exists() {
            // Re-installing over our own shim: keep whatever it chained to.
            chain = existing_chain(&target);
        }

        std::fs::write(&target, shim_source(hook, &binary, chain.as_deref()))
            .with_context(|| format!("cannot write {}", target.display()))?;
        make_executable(&target)?;
        writeln!(out, "  installed {}", target.display())?;
    }

    if scope != Scope::Local {
        set_hooks_path(scope, &dir, out)?;
    }

    writeln!(out, "gitscale hooks installed ({})", scope.label())?;
    Ok(())
}

/// Read the chained hook out of a shim we previously wrote.
fn existing_chain(shim: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(shim).ok()?;
    let line = text.lines().find(|l| l.starts_with("CHAIN='"))?;
    let value = line.trim_start_matches("CHAIN='").trim_end_matches('\'');
    (!value.is_empty()).then(|| PathBuf::from(value))
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
            effective_hooks_dir(&repo)?.0
        }
        Scope::Global | Scope::System => managed_hooks_dir(scope)?,
    };

    let mut removed = 0;
    for hook in HOOKS {
        let target = dir.join(hook);
        if !target.exists() || !is_shim(&target) {
            continue;
        }
        let chain = existing_chain(&target);
        std::fs::remove_file(&target)
            .with_context(|| format!("cannot remove {}", target.display()))?;
        removed += 1;
        // Put the repo's own hook back where git expects it.
        if let Some(displaced) = chain {
            if displaced.exists() {
                std::fs::rename(&displaced, &target)
                    .with_context(|| format!("cannot restore {}", target.display()))?;
                writeln!(out, "  restored {}", target.display())?;
                continue;
            }
        }
        writeln!(out, "  removed {}", target.display())?;
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
    let (dir, repo_chose_it) = effective_hooks_dir(&repo)?;
    writeln!(out, "\nrepo     {}", repo.display())?;
    writeln!(out, "hooks    {}", dir.display())?;

    if repo_chose_it {
        let shadowed = git_config_get(Some(Scope::Global), "core.hooksPath", None).is_some()
            || git_config_get(Some(Scope::System), "core.hooksPath", None).is_some();
        if shadowed {
            writeln!(
                out,
                "\nThis repo sets its own core.hooksPath, which overrides the global one.\n\
                 A global gitscale install does not run here — use `gitscale hook install --local`."
            )?;
        }
    }

    for hook in HOOKS {
        let target = dir.join(hook);
        let state = if !target.exists() {
            "not installed"
        } else if is_shim(&target) {
            "gitscale"
        } else {
            "other (not gitscale)"
        };
        writeln!(out, "  {:<14} {}", hook, state)?;
    }

    if let Some(note) = read_breadcrumb(&repo)? {
        writeln!(out, "\nLast hook-triggered pull FAILED:\n  {}", note.trim())?;
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

/// Run the pull a git hook asked for.
///
/// A failure here is reported three ways, because none alone is sufficient: on
/// stderr (which reaches the user through `git clone`), in a breadcrumb file so
/// a later, more confusing failure can be traced back to this one, and — under
/// the `fail` policy — in the exit status. The exit status is the weakest of
/// the three: git collapses any non-zero hook exit to 1 and reports it as the
/// *checkout* failing, which it did not.
pub fn run(
    hook: &str,
    root: Option<&Path>,
    verbose: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    if !HOOKS.contains(&hook) {
        bail!(
            "unknown hook '{}' (gitscale installs: {})",
            hook,
            HOOKS.join(", ")
        );
    }

    let repo = worktree_root(root)?;
    // The shim checks this too; re-check so a hand-run `hook run` behaves.
    if !repo.join(CONFIG_FILENAME).is_file() {
        return Ok(());
    }

    let policy = {
        let config_path = repo.join(CONFIG_FILENAME);
        let configured = load_config_optional(&config_path).and_then(|c| c.hooks.on_pull_error);
        OnHookError::resolved(configured)
    };

    let result = crate::commands::pull::run(Some(&repo), &[], verbose, interactive, out, err);

    let breadcrumb = breadcrumb_path(&repo)?;
    match result {
        Ok(()) => {
            let _ = std::fs::remove_file(&breadcrumb);
            Ok(())
        }
        Err(e) => {
            let note = format!(
                "{}: `gitscale pull` triggered by the {} hook failed: {}\n",
                now_stamp(),
                hook,
                e
            );
            let _ = std::fs::write(&breadcrumb, &note);
            writeln!(err, "gitscale: {} hook — pull failed: {}", hook, e)?;
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

fn now_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

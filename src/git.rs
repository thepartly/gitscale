use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::ci;
use crate::config::RepoEntry;
use crate::share::Reference;

pub fn is_ci() -> bool {
    std::env::var("CI")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true"))
        .unwrap_or(false)
}

fn run_git(args: &[&str], cwd: Option<&Path>, check: bool) -> Result<std::process::Output> {
    let mut cmd = Command::new("git");
    // Under CI, teach git how to authenticate to the CI server. Scoped to that
    // one host, and carrying the name of the token variable rather than the
    // token, so nothing secret reaches argv or a config file.
    if let Some(auth) = ci::active() {
        cmd.args(auth.git_config_args());
    }
    cmd.args(args);
    cmd.stdin(Stdio::null());
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    // Marks every git call gitscale makes, so an installed gitscale git hook
    // can tell re-entry from a genuine user operation and bail out. Without
    // it, a hook that runs `gitscale pull` recurses without bound.
    cmd.env("GITSCALE_HOOK", "1");
    cmd.env("GIT_ASKPASS", "");
    cmd.env("SSH_ASKPASS", "");
    cmd.env("SSH_ASKPASS_REQUIRE", "never");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd
        .output()
        .with_context(|| format!("failed to run: git {}", args.join(" ")))?;
    if check && !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("{}", git_failure(&stderr, ci::active()));
    }
    Ok(output)
}

/// Pick the most relevant line from git/ssh stderr output. Advisory notices
/// (e.g. openssh's post-quantum key exchange warning) print early, ahead of
/// the actual failure, so a plain "first line" pick reports the wrong thing.
fn git_error_line(stderr: &str) -> &str {
    let trimmed = stderr.trim();
    trimmed
        .lines()
        .find(|l| l.contains("fatal:") || l.contains("error:"))
        .or_else(|| trimmed.lines().last())
        .unwrap_or("unknown error")
}

/// The message to report for a failed git call. A 403 from the CI server is
/// the failure people actually hit: the token is valid but the target project
/// has not allowed this one to read it, and git's own wording gives no clue.
fn git_failure(stderr: &str, auth: Option<&crate::ci::CiAuth>) -> String {
    let line = git_error_line(stderr);
    match auth {
        Some(auth) if stderr.contains("403") => {
            format!("{}\nhint: {}", line, auth.forbidden_hint())
        }
        _ => line.to_string(),
    }
}

fn stdout_str(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

// ---------------------------------------------------------------------------
// Core operations
// ---------------------------------------------------------------------------

/// True if `revision` names a commit rather than a branch or tag. `git clone
/// --branch` accepts only branch and tag names, so a SHA-pinned entry cannot
/// be cloned shallowly the usual way.
fn looks_like_sha(revision: &str) -> bool {
    revision.len() >= 7 && revision.len() <= 64 && revision.chars().all(|c| c.is_ascii_hexdigit())
}

/// Shallow-clone a repo pinned to a commit SHA: init an empty repo and fetch
/// just that commit. Returns `false` if the remote refused to serve the commit
/// (not every server allows fetching an arbitrary SHA), leaving the caller to
/// fall back to a full clone.
fn shallow_clone_at_sha(entry: &RepoEntry, dest: &Path, verbose: bool) -> Result<bool> {
    let progress = if verbose { "--progress" } else { "--quiet" };
    fs::create_dir_all(dest).with_context(|| format!("cannot create {}", dest.display()))?;
    run_git(&["init", "--quiet"], Some(dest), true)?;
    run_git(
        &["remote", "add", "origin", &remote_url(entry)],
        Some(dest),
        true,
    )?;
    let fetched = run_git(
        &["fetch", "--depth", "1", progress, "origin", &entry.revision],
        Some(dest),
        false,
    )?;
    if !fetched.status.success() {
        return Ok(false);
    }
    run_git(
        &["checkout", "--detach", progress, "FETCH_HEAD"],
        Some(dest),
        true,
    )?;
    Ok(true)
}

/// Clone `entry` into `root`.
///
/// `reference` names a copy already on this machine whose objects the new
/// clone may take instead of downloading them. See [`crate::share`] for how
/// one is found; passing `None` always produces an ordinary clone.
pub fn clone_repo(
    entry: &RepoEntry,
    root: &Path,
    verbose: bool,
    shallow: bool,
    reference: Option<&Reference>,
) -> Result<()> {
    if entry.is_artefact() {
        return Ok(());
    }
    let dest = root.join(&entry.directory);
    if dest.exists() {
        bail!("Directory already exists: {}", dest.display());
    }

    if shallow && looks_like_sha(&entry.revision) {
        // This path builds the repository with `init` + a one-commit `fetch`
        // rather than `clone`, and there is no `--reference` for fetch. Little
        // is lost: a depth-1 fetch of a single commit transfers about as much
        // as wiring up an alternate would save.
        if !shallow_clone_at_sha(entry, &dest, verbose)? {
            // Remote would not serve the bare commit; retry unshallowed.
            fs::remove_dir_all(&dest)
                .with_context(|| format!("cannot clean up {}", dest.display()))?;
            return clone_repo(entry, root, verbose, false, reference);
        }
    } else {
        let dest_str = dest.to_string_lossy().to_string();
        let url = remote_url(entry);
        let reference_path = reference.map(|r| r.path.to_string_lossy().to_string());
        let mut args: Vec<&str> = vec!["clone", &url, &dest_str];
        if shallow && !entry.revision.is_empty() {
            args.extend_from_slice(&["--depth", "1", "--branch", &entry.revision]);
        } else if shallow {
            args.extend_from_slice(&["--depth", "1"]);
        }
        if let (Some(reference), Some(path)) = (reference, reference_path.as_deref()) {
            args.extend_from_slice(&["--reference", path]);
            if reference.dissociate {
                args.push("--dissociate");
            }
        }
        if verbose {
            args.push("--progress");
        } else {
            args.push("--quiet");
        }
        run_git(&args, None, true)?;

        if !shallow && !entry.revision.is_empty() {
            checkout_revision(entry, root)?;
        }
    }
    if entry.is_readonly() {
        apply_readonly(&dest)?;
    }
    Ok(())
}

/// Ensure an existing clone's `origin` remote URL matches the configured URL.
/// Returns `true` if the remote was updated.
pub fn reconcile_remote(entry: &RepoEntry, root: &Path) -> Result<bool> {
    if entry.is_artefact() {
        return Ok(false);
    }
    let dest = root.join(&entry.directory);
    if !dest.exists() {
        return Ok(false);
    }
    let current = run_git(&["remote", "get-url", "origin"], Some(&dest), false)?;
    if !current.status.success() {
        return Ok(false);
    }
    let url = remote_url(entry);
    if stdout_str(&current) == url {
        return Ok(false);
    }
    run_git(&["remote", "set-url", "origin", &url], Some(&dest), true)?;
    Ok(true)
}

/// The `origin` URL of the repository `dir` belongs to, or `None` if it is not
/// in one or has no such remote. Resolved by git rather than by looking for a
/// `.git` directory, so a path inside a repository answers for that repository.
pub fn origin_url(dir: &Path) -> Option<String> {
    let output = run_git(&["remote", "get-url", "origin"], Some(dir), false).ok()?;
    if !output.status.success() {
        return None;
    }
    let url = stdout_str(&output);
    (!url.is_empty()).then_some(url)
}

/// The URL to use as `origin` for `entry`: the configured one, unless CI
/// credentials cover its host and can fetch it over HTTPS instead.
pub fn remote_url(entry: &RepoEntry) -> String {
    ci::remote_url(&entry.repo_url)
}

/// Repoint an existing clone at the CI server's HTTPS URL before a network
/// operation. A no-op outside CI, and for any host the CI server doesn't own:
/// a workspace cloned over SSH (restored from cache, or checked out by the
/// runner itself) otherwise keeps failing on an SSH key the job doesn't have.
fn ensure_ci_remote(entry: &RepoEntry, dest: &Path) -> Result<()> {
    let Some(auth) = ci::active() else {
        return Ok(());
    };
    let Some(url) = auth.remote_url(&entry.repo_url) else {
        return Ok(());
    };
    let current = run_git(&["remote", "get-url", "origin"], Some(dest), false)?;
    if !current.status.success() || stdout_str(&current) == url {
        return Ok(());
    }
    run_git(&["remote", "set-url", "origin", &url], Some(dest), true)?;
    Ok(())
}

pub fn checkout_revision(entry: &RepoEntry, root: &Path) -> Result<()> {
    let dest = root.join(&entry.directory);
    // Try branch checkout first, then detached HEAD for tags/SHAs
    let result = run_git(&["checkout", &entry.revision], Some(&dest), false)?;
    if !result.status.success() {
        let result = run_git(
            &[
                "rev-parse",
                "--verify",
                &format!("{}^{{commit}}", entry.revision),
            ],
            Some(&dest),
            false,
        )?;
        if !result.status.success() {
            bail!(
                "revision '{}' does not exist in {}",
                entry.revision,
                entry.directory
            );
        }
        run_git(
            &["checkout", "--detach", &entry.revision],
            Some(&dest),
            true,
        )?;
    }
    Ok(())
}

pub fn is_shallow(dest: &Path) -> bool {
    run_git(&["rev-parse", "--is-shallow-repository"], Some(dest), false)
        .map(|o| stdout_str(&o) == "true")
        .unwrap_or(false)
}

pub fn fetch_repo(entry: &RepoEntry, root: &Path) -> Result<()> {
    let dest = root.join(&entry.directory);
    ensure_ci_remote(entry, &dest)?;
    if is_shallow(&dest) {
        run_git(&["fetch", "--depth", "1", "--quiet"], Some(&dest), true)?;
    } else {
        run_git(&["fetch", "--all", "--quiet"], Some(&dest), true)?;
    }
    Ok(())
}

pub fn apply_readonly(dest: &Path) -> Result<()> {
    for entry in walkdir(dest) {
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }
        if is_inside_dotgit(dest, path) {
            continue;
        }
        if path.is_file() {
            let meta = fs::metadata(path)?;
            let mut perms = meta.permissions();
            let mode = perms.mode();
            perms.set_mode(mode & !(0o222)); // remove S_IWUSR | S_IWGRP | S_IWOTH
            fs::set_permissions(path, perms)?;
        }
    }
    Ok(())
}

pub fn restore_writable(dest: &Path) -> Result<()> {
    for entry in walkdir(dest) {
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }
        if is_inside_dotgit(dest, path) {
            continue;
        }
        if path.is_file() {
            let meta = fs::metadata(path)?;
            let mut perms = meta.permissions();
            let mode = perms.mode();
            perms.set_mode(mode | 0o200); // add S_IWUSR
            fs::set_permissions(path, perms)?;
        }
    }
    Ok(())
}

fn is_inside_dotgit(root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.components().any(|c| c.as_os_str() == ".git")
}

struct WalkEntry {
    path: std::path::PathBuf,
}

impl WalkEntry {
    fn path(&self) -> &Path {
        &self.path
    }
}

fn walkdir(root: &Path) -> Vec<WalkEntry> {
    let mut result = Vec::new();
    walkdir_recursive(root, root, &mut result);
    result
}

fn walkdir_recursive(root: &Path, dir: &Path, result: &mut Vec<WalkEntry>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Skip .git directories
        if path.is_dir() && path.file_name().map(|n| n == ".git").unwrap_or(false) {
            continue;
        }
        result.push(WalkEntry { path: path.clone() });
        if path.is_dir() && !path.is_symlink() {
            walkdir_recursive(root, &path, result);
        }
    }
}

pub fn sync_repo(entry: &RepoEntry, root: &Path, verbose: bool, shallow: bool) -> Result<()> {
    if entry.is_artefact() {
        return Ok(());
    }
    let dest = root.join(&entry.directory);
    if !dest.exists() {
        return clone_repo(entry, root, verbose, shallow, None);
    }

    if entry.is_readonly() {
        restore_writable(&dest)?;
    }

    let result = (|| -> Result<()> {
        fetch_repo(entry, root)?;
        if is_shallow(&dest) {
            run_git(&["reset", "--hard", "@{upstream}"], Some(&dest), false)?;
        } else {
            checkout_revision(entry, root)?;
            let head_ref = get_current_ref(entry, root)?;
            if !head_ref.is_empty() && !is_detached(entry, root) {
                run_git(&["pull", "--ff-only", "--quiet"], Some(&dest), false)?;
            }
        }
        Ok(())
    })();

    if entry.is_readonly() {
        apply_readonly(&dest)?;
    }
    result
}

/// Bring `entry` up to date, cloning it first if it is not there yet — which
/// is the usual case in a freshly created worktree, where the hook-triggered
/// pull is the first thing to run. `reference` is used only for that clone.
pub fn pull_repo(
    entry: &RepoEntry,
    root: &Path,
    verbose: bool,
    shallow: bool,
    reference: Option<&Reference>,
) -> Result<()> {
    if entry.is_artefact() {
        return Ok(());
    }
    let dest = root.join(&entry.directory);
    if !dest.exists() {
        return clone_repo(entry, root, verbose, shallow, reference);
    }

    if entry.is_readonly() {
        restore_writable(&dest)?;
    }

    ensure_ci_remote(entry, &dest)?;

    let result = (|| -> Result<()> {
        if is_shallow(&dest) {
            if looks_like_sha(&entry.revision) {
                // Detached at a commit: there is no @{upstream} to reset to.
                run_git(
                    &[
                        "fetch",
                        "--depth",
                        "1",
                        "--quiet",
                        "origin",
                        &entry.revision,
                    ],
                    Some(&dest),
                    true,
                )?;
                run_git(
                    &["reset", "--hard", "--quiet", "FETCH_HEAD"],
                    Some(&dest),
                    true,
                )?;
            } else {
                run_git(&["fetch", "--depth", "1", "--quiet"], Some(&dest), true)?;
                run_git(&["reset", "--hard", "@{upstream}"], Some(&dest), false)?;
            }
        } else {
            let current = get_current_ref(entry, root)?;
            if current != entry.revision {
                checkout_revision(entry, root)?;
            }
            run_git(&["pull", "--ff-only", "--quiet"], Some(&dest), false)?;
        }
        Ok(())
    })();

    if entry.is_readonly() {
        apply_readonly(&dest)?;
    }
    result
}

pub fn push_repo(entry: &RepoEntry, root: &Path, _verbose: bool) -> Result<()> {
    if entry.is_artefact() || entry.is_readonly() {
        return Ok(());
    }
    let dest = root.join(&entry.directory);
    if !dest.exists() {
        return Ok(());
    }
    ensure_ci_remote(entry, &dest)?;
    run_git(&["push", "--quiet"], Some(&dest), true)?;
    Ok(())
}

/// Stage all changes (including untracked) and commit them with `message`.
/// Returns `Ok(true)` if a commit was created, `Ok(false)` if the working tree
/// was already clean (nothing to commit).
pub fn commit_path(dir: &Path, message: &str) -> Result<bool> {
    if !dir.exists() {
        return Ok(false);
    }
    // Nothing to commit if the working tree is clean.
    let porcelain = run_git(&["status", "--porcelain"], Some(dir), true)?;
    if stdout_str(&porcelain).is_empty() {
        return Ok(false);
    }
    run_git(&["add", "-A"], Some(dir), true)?;
    run_git(&["commit", "-m", message], Some(dir), true)?;
    Ok(true)
}

/// Returns true when `dir` is the top level of its own git repository (not
/// merely nested inside some ancestor git repo).
pub fn is_repo_root(dir: &Path) -> bool {
    run_git(&["rev-parse", "--show-toplevel"], Some(dir), false)
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let top = std::path::PathBuf::from(stdout_str(&o));
            match (fs::canonicalize(&top), fs::canonicalize(dir)) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            }
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RepoStatus {
    pub directory: String,
    pub exists: bool,
    pub current_ref: String,
    pub expected_ref: String,
    pub is_clean: bool,
    pub is_detached: bool,
    pub ahead: i32,
    pub behind: i32,
    pub mode: String,
    pub is_stale: bool,
    pub is_symlink: bool,
    pub symlink_target: String,
    pub has_unlinked: bool,
    pub has_unlinked_modified: bool,
}

pub fn get_current_ref(entry: &RepoEntry, root: &Path) -> Result<String> {
    let dest = root.join(&entry.directory);
    let result = run_git(&["symbolic-ref", "--short", "HEAD"], Some(&dest), false)?;
    if result.status.success() {
        return Ok(stdout_str(&result));
    }
    let result = run_git(&["rev-parse", "--short", "HEAD"], Some(&dest), true)?;
    Ok(stdout_str(&result))
}

pub fn is_detached(entry: &RepoEntry, root: &Path) -> bool {
    let dest = root.join(&entry.directory);
    run_git(&["symbolic-ref", "HEAD"], Some(&dest), false)
        .map(|o| !o.status.success())
        .unwrap_or(true)
}

pub fn is_clean(entry: &RepoEntry, root: &Path) -> bool {
    let dest = root.join(&entry.directory);
    run_git(&["status", "--porcelain"], Some(&dest), false)
        .map(|o| stdout_str(&o).is_empty())
        .unwrap_or(false)
}

pub fn get_ahead_behind(entry: &RepoEntry, root: &Path) -> (i32, i32) {
    let dest = root.join(&entry.directory);
    let result = run_git(
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        Some(&dest),
        false,
    );
    match result {
        Ok(o) if o.status.success() => {
            let s = stdout_str(&o);
            let parts: Vec<&str> = s.split_whitespace().collect();
            if parts.len() == 2 {
                let ahead = parts[0].parse().unwrap_or(0);
                let behind = parts[1].parse().unwrap_or(0);
                return (ahead, behind);
            }
            (0, 0)
        }
        _ => (0, 0),
    }
}

fn is_stale(dest: &Path) -> bool {
    let local = run_git(&["rev-parse", "HEAD"], Some(dest), false);
    let remote = run_git(&["rev-parse", "@{upstream}"], Some(dest), false);
    match (local, remote) {
        (Ok(l), Ok(r)) if l.status.success() && r.status.success() => {
            stdout_str(&l) != stdout_str(&r)
        }
        _ => false,
    }
}

pub fn get_repo_status(entry: &RepoEntry, root: &Path) -> RepoStatus {
    let dest = root.join(&entry.directory);
    let symlink = dest
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if !dest.exists() {
        return RepoStatus {
            directory: entry.directory.clone(),
            exists: false,
            current_ref: String::new(),
            expected_ref: entry.revision.clone(),
            is_clean: true,
            is_detached: false,
            ahead: 0,
            behind: 0,
            mode: entry.mode.to_string(),
            is_stale: false,
            is_symlink: symlink,
            symlink_target: String::new(),
            has_unlinked: false,
            has_unlinked_modified: false,
        };
    }
    if symlink {
        let target = fs::read_link(&dest)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let resolved = dest.canonicalize().unwrap_or_default();
        let ref_str = run_git(&["symbolic-ref", "--short", "HEAD"], Some(&resolved), false)
            .ok()
            .filter(|o| o.status.success())
            .map(|o| stdout_str(&o))
            .or_else(|| {
                run_git(&["rev-parse", "--short", "HEAD"], Some(&resolved), false)
                    .ok()
                    .map(|o| stdout_str(&o))
            })
            .unwrap_or_default();
        return RepoStatus {
            directory: entry.directory.clone(),
            exists: true,
            current_ref: ref_str,
            expected_ref: entry.revision.clone(),
            is_clean: true,
            is_detached: false,
            ahead: 0,
            behind: 0,
            mode: entry.mode.to_string(),
            is_stale: false,
            is_symlink: true,
            symlink_target: target,
            has_unlinked: false,
            has_unlinked_modified: false,
        };
    }

    let current = get_current_ref(entry, root).unwrap_or_default();
    let detached = is_detached(entry, root);
    let clean = is_clean(entry, root);
    let shallow = is_shallow(&dest);

    if shallow {
        let stale = is_stale(&dest);
        return RepoStatus {
            directory: entry.directory.clone(),
            exists: true,
            current_ref: current,
            expected_ref: entry.revision.clone(),
            is_clean: clean,
            is_detached: detached,
            ahead: 0,
            behind: 0,
            mode: entry.mode.to_string(),
            is_stale: stale,
            is_symlink: false,
            symlink_target: String::new(),
            has_unlinked: false,
            has_unlinked_modified: false,
        };
    }

    let (ahead, behind) = get_ahead_behind(entry, root);
    RepoStatus {
        directory: entry.directory.clone(),
        exists: true,
        current_ref: current,
        expected_ref: entry.revision.clone(),
        is_clean: clean,
        is_detached: detached,
        ahead,
        behind,
        mode: entry.mode.to_string(),
        is_stale: false,
        is_symlink: false,
        symlink_target: String::new(),
        has_unlinked: false,
        has_unlinked_modified: false,
    }
}

pub fn get_self_status(root: &Path) -> Option<RepoStatus> {
    let result = run_git(&["rev-parse", "--is-inside-work-tree"], Some(root), false).ok()?;
    if !result.status.success() {
        return None;
    }

    let ref_result = run_git(&["symbolic-ref", "--short", "HEAD"], Some(root), false).ok()?;
    let (current_ref, detached) = if ref_result.status.success() {
        (stdout_str(&ref_result), false)
    } else {
        let rev = run_git(&["rev-parse", "--short", "HEAD"], Some(root), false).ok()?;
        (stdout_str(&rev), true)
    };

    let clean = run_git(&["status", "--porcelain"], Some(root), false)
        .map(|o| stdout_str(&o).is_empty())
        .unwrap_or(false);

    let (ahead, behind) = {
        let ab = run_git(
            &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
            Some(root),
            false,
        );
        match ab {
            Ok(o) if o.status.success() => {
                let s = stdout_str(&o);
                let parts: Vec<&str> = s.split_whitespace().collect();
                if parts.len() == 2 {
                    (parts[0].parse().unwrap_or(0), parts[1].parse().unwrap_or(0))
                } else {
                    (0, 0)
                }
            }
            _ => (0, 0),
        }
    };

    Some(RepoStatus {
        directory: ".".to_string(),
        exists: true,
        current_ref,
        expected_ref: String::new(),
        is_clean: clean,
        is_detached: detached,
        ahead,
        behind,
        mode: String::new(),
        is_stale: false,
        is_symlink: false,
        symlink_target: String::new(),
        has_unlinked: false,
        has_unlinked_modified: false,
    })
}

pub fn get_artefact_status(entry: &RepoEntry, root: &Path) -> RepoStatus {
    let dest = root.join(&entry.directory);
    let symlink = dest
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if symlink {
        let target = fs::read_link(&dest)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        return RepoStatus {
            directory: entry.directory.clone(),
            exists: true,
            current_ref: String::new(),
            expected_ref: entry.revision.clone(),
            is_clean: true,
            is_detached: false,
            ahead: 0,
            behind: 0,
            mode: entry.mode.to_string(),
            is_stale: false,
            is_symlink: true,
            symlink_target: target,
            has_unlinked: false,
            has_unlinked_modified: false,
        };
    }

    let exists = dest.is_dir() && dest.join(".etag").is_file();
    let mut behind = 0;
    let etag_file = dest.join(".etag");
    let etag_remote_file = dest.join(".etag-remote");
    if etag_file.is_file() && etag_remote_file.is_file() {
        let local = fs::read_to_string(&etag_file)
            .unwrap_or_default()
            .trim()
            .to_string();
        let remote = fs::read_to_string(&etag_remote_file)
            .unwrap_or_default()
            .trim()
            .to_string();
        if !local.is_empty() && !remote.is_empty() && local != remote {
            behind = 1;
        }
    }

    RepoStatus {
        directory: entry.directory.clone(),
        exists,
        current_ref: if exists {
            "artefact".to_string()
        } else {
            String::new()
        },
        expected_ref: entry.revision.clone(),
        is_clean: true,
        is_detached: false,
        ahead: 0,
        behind,
        mode: entry.mode.to_string(),
        is_stale: false,
        is_symlink: false,
        symlink_target: String::new(),
        has_unlinked: false,
        has_unlinked_modified: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{git_error_line, git_failure};
    use crate::ci::CiAuth;
    use std::collections::HashMap;

    #[test]
    fn picks_fatal_line_over_leading_warning() {
        let stderr = "** WARNING: connection is not using a post-quantum key exchange algorithm.\n\
                       git@gitlab.example.com: Permission denied (publickey).\n\
                       fatal: Could not read from remote repository.\n\
                       \n\
                       Please make sure you have the correct access rights\n\
                       and the repository exists.";
        assert_eq!(
            git_error_line(stderr),
            "fatal: Could not read from remote repository."
        );
    }

    #[test]
    fn falls_back_to_last_line_when_no_fatal_marker() {
        let stderr = "Cloning into 'repo'...\nsomething went sideways";
        assert_eq!(git_error_line(stderr), "something went sideways");
    }

    #[test]
    fn falls_back_to_unknown_error_when_empty() {
        assert_eq!(git_error_line(""), "unknown error");
    }

    #[test]
    fn explains_a_403_from_the_ci_server() {
        let auth = CiAuth::from_map(&HashMap::from([
            ("CI_JOB_TOKEN", "tok"),
            ("CI_SERVER_URL", "https://gitlab.example.com"),
        ]))
        .unwrap();
        let stderr = "fatal: unable to access \
                      'https://gitlab.example.com/acme/payments.git/': \
                      The requested URL returned error: 403";
        let message = git_failure(stderr, Some(&auth));
        assert!(message.starts_with("fatal: unable to access"));
        assert!(message.contains("Job token permissions"));
        // Without CI credentials there is nothing useful to add.
        assert!(!git_failure(stderr, None).contains("hint:"));
    }

    #[test]
    fn leaves_unrelated_failures_unannotated() {
        let auth = CiAuth::from_map(&HashMap::from([
            ("CI_JOB_TOKEN", "tok"),
            ("CI_SERVER_URL", "https://gitlab.example.com"),
        ]))
        .unwrap();
        let stderr = "fatal: repository 'https://gitlab.example.com/x.git/' not found";
        assert!(!git_failure(stderr, Some(&auth)).contains("hint:"));
    }
}

use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::config::RepoEntry;

pub fn is_ci() -> bool {
    std::env::var("CI")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true"))
        .unwrap_or(false)
}

fn run_git(args: &[&str], cwd: Option<&Path>, check: bool) -> Result<std::process::Output> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    cmd.stdin(Stdio::null());
    cmd.env("GIT_TERMINAL_PROMPT", "0");
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
        let msg = stderr.trim().lines().next().unwrap_or("unknown error");
        bail!("{}", msg);
    }
    Ok(output)
}

fn stdout_str(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

// ---------------------------------------------------------------------------
// Core operations
// ---------------------------------------------------------------------------

pub fn clone_repo(entry: &RepoEntry, root: &Path, verbose: bool, shallow: bool) -> Result<()> {
    if entry.is_artefact() {
        return Ok(());
    }
    let dest = root.join(&entry.directory);
    if dest.exists() {
        bail!("Directory already exists: {}", dest.display());
    }

    let dest_str = dest.to_string_lossy().to_string();
    let mut args: Vec<&str> = vec!["clone", &entry.repo_url, &dest_str];
    if shallow {
        args.extend_from_slice(&["--depth", "1", "--branch", &entry.revision]);
    }
    if verbose {
        args.push("--progress");
    } else {
        args.push("--quiet");
    }
    run_git(&args, None, true)?;

    if !shallow {
        checkout_revision(entry, root)?;
    }
    if entry.is_readonly() {
        apply_readonly(&dest)?;
    }
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
        return clone_repo(entry, root, verbose, shallow);
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

pub fn pull_repo(entry: &RepoEntry, root: &Path, verbose: bool, shallow: bool) -> Result<()> {
    if entry.is_artefact() {
        return Ok(());
    }
    let dest = root.join(&entry.directory);
    if !dest.exists() {
        return clone_repo(entry, root, verbose, shallow);
    }

    if entry.is_readonly() {
        restore_writable(&dest)?;
    }

    let result = (|| -> Result<()> {
        if is_shallow(&dest) {
            run_git(&["fetch", "--depth", "1", "--quiet"], Some(&dest), true)?;
            run_git(&["reset", "--hard", "@{upstream}"], Some(&dest), false)?;
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
    run_git(&["push", "--quiet"], Some(&dest), true)?;
    Ok(())
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
    })
}

pub fn get_artefact_status(entry: &RepoEntry, root: &Path) -> RepoStatus {
    let dest = root.join(&entry.directory);
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
    }
}

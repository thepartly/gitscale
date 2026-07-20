use anyhow::Result;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::config::{find_config, load_config, load_config_optional, CONFIG_FILENAME};
use crate::hooks;
use crate::resolve::{create_symlinks, resolve_recursive};

pub fn run(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    force: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    crate::commands::clone::run(root, names, verbose, interactive, out, err)?;
    reconcile_remotes(root, names, out)?;
    // pull runs its own post_sync hook, skip it here to avoid double-run
    crate::commands::pull::run_no_hooks(root, names, verbose, interactive, out, err)?;

    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;

    // Restore symlinks for unlinked clones and remove orphaned links before
    // pushing, so local hygiene isn't blocked by a remote/auth failure.
    relink(&config.repos, &config_root, force, out)?;

    crate::commands::push::run(root, names, verbose, interactive, out, err)?;

    hooks::run_post_sync(&config.hooks, &config_root, verbose, out)?;
    Ok(())
}

/// Update each existing clone's `origin` remote to match the configured URL.
fn reconcile_remotes(root: Option<&Path>, names: &[String], out: &mut dyn Write) -> Result<()> {
    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;
    let selected = crate::commands::clone::filter_entries(&config.repos, names)?;

    let mut header_done = false;
    for entry in &selected {
        if crate::git::reconcile_remote(entry, &config_root)? {
            if !header_done {
                writeln!(out, "Reconciling remotes...")?;
                header_done = true;
            }
            writeln!(out, "  update  {} -> {}", entry.directory, entry.repo_url)?;
        }
    }
    Ok(())
}

fn relink(
    repos: &[crate::config::RepoEntry],
    config_root: &Path,
    force: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let (symlinks, _) = resolve_recursive(repos, config_root)?;

    // Remove orphaned symlinks (links whose dep was removed from config).
    // Broken orphans are always safe to remove; orphans that still resolve to a
    // valid checkout are only removed with --force.
    let orphans = crate::resolve::find_orphan_links(repos, config_root, &symlinks);
    let mut orphan_skipped = 0usize;
    for orphan in &orphans {
        let link_abs = config_root.join(&orphan.link_path);
        if orphan.broken || force {
            std::fs::remove_file(&link_abs)?;
            writeln!(out, "  unlink  {} (orphan)", orphan.link_path.display())?;
        } else {
            writeln!(
                out,
                "  skip  {} (orphan with valid target, use --force to remove)",
                orphan.link_path.display()
            )?;
            orphan_skipped += 1;
        }
    }

    let mut skipped = 0usize;

    for sym in &symlinks {
        let link_abs = config_root.join(&sym.link_path);
        // Only act on directories that should be symlinks but aren't
        if !link_abs.exists()
            || link_abs
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        {
            continue;
        }

        let modified = is_tree_modified(&link_abs);
        if modified && !force {
            writeln!(
                out,
                "  skip  {} (modified, use --force to relink)",
                sym.link_path.display()
            )?;
            skipped += 1;
            continue;
        }

        // Remove the real directory and let create_symlinks restore it
        std::fs::remove_dir_all(&link_abs)?;
        writeln!(out, "  relink  {}", sym.link_path.display())?;
    }

    // Restore symlinks for any that were removed
    create_symlinks(&symlinks, config_root, out)?;

    if skipped > 0 || orphan_skipped > 0 {
        let mut parts = Vec::new();
        if skipped > 0 {
            parts.push(format!(
                "{} unlinked repo(s) with local modifications",
                skipped
            ));
        }
        if orphan_skipped > 0 {
            parts.push(format!(
                "{} orphaned link(s) with valid targets",
                orphan_skipped
            ));
        }
        anyhow::bail!("{} (use --force to override)", parts.join(" and "));
    }

    Ok(())
}

fn is_tree_modified(path: &Path) -> bool {
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(path)
        .stdin(Stdio::null())
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);
    if dirty {
        return true;
    }

    let ahead = Command::new("git")
        .args(["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])
        .current_dir(path)
        .stdin(Stdio::null())
        .output()
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let parts: Vec<&str> = s.split_whitespace().collect();
            parts
                .first()
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(0)
                > 0
        })
        .unwrap_or(false);
    if ahead {
        return true;
    }

    let child_config_path = path.join(CONFIG_FILENAME);
    if let Some(child_config) = load_config_optional(&child_config_path) {
        for entry in &child_config.repos {
            if entry.is_artefact() {
                continue;
            }
            let child_path = path.join(&entry.directory);
            if child_path.is_dir() && !child_path.is_symlink() {
                if is_tree_modified(&child_path) {
                    return true;
                }
            }
        }
    }

    false
}

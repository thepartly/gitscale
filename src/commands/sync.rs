use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::config::{filter_entries, load_workspace, GitScaleConfig};
use crate::git::is_tree_modified;
use crate::hooks;
use crate::resolve::{create_symlinks, resolve_recursive};

#[allow(clippy::too_many_arguments)]
pub fn run(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    no_cache: bool,
    force: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    crate::commands::clone::run(root, names, verbose, no_cache, interactive, out, err)?;
    reconcile_remotes(&config, &config_root, names, out)?;
    // pull runs its own post_sync hook, skip it here to avoid double-run
    crate::commands::pull::run_no_hooks(root, names, verbose, no_cache, interactive, out, err)?;

    // Restore symlinks for unlinked clones and remove orphaned links before
    // pushing, so local hygiene isn't blocked by a remote/auth failure.
    relink(&config.repos, names, &config_root, force, out)?;

    crate::commands::push::run(root, names, verbose, interactive, out, err)?;

    hooks::run_post_sync(&config.hooks, &config_root, verbose, out)?;
    Ok(())
}

/// Update each existing clone's `origin` remote to match the configured URL.
fn reconcile_remotes(
    config: &GitScaleConfig,
    config_root: &Path,
    names: &[String],
    out: &mut dyn Write,
) -> Result<()> {
    let selected = filter_entries(&config.repos, names)?;

    let mut header_done = false;
    for entry in &selected {
        if crate::git::reconcile_remote(entry, config_root)? {
            if !header_done {
                writeln!(out, "Reconciling remotes...")?;
                header_done = true;
            }
            writeln!(
                out,
                "  update  {} -> {}",
                entry.directory,
                crate::git::remote_url(entry)
            )?;
        }
    }
    Ok(())
}

fn relink(
    repos: &[crate::config::RepoEntry],
    names: &[String],
    config_root: &Path,
    force: bool,
    out: &mut dyn Write,
) -> Result<()> {
    // Resolved against every repo, so dependencies are checked as a whole;
    // acted on only inside the repos named, since a link belongs to the repo
    // it sits in and that repo was not asked to sync.
    let (all_symlinks, _) = resolve_recursive(repos, config_root)?;
    let selected = crate::config::filter_entries(repos, names)?;
    let symlinks: Vec<_> = all_symlinks
        .iter()
        .filter(|sym| {
            crate::resolve::owning_entry(&sym.link_path, repos)
                .is_some_and(|owner| selected.iter().any(|e| e.directory == owner.directory))
        })
        .cloned()
        .collect();

    // Remove orphaned symlinks (links whose dep was removed from config).
    // Broken orphans are always safe to remove; orphans that still resolve to a
    // valid checkout are only removed with --force.
    let orphans = crate::resolve::find_orphan_links(&selected, config_root, &all_symlinks);
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
    create_symlinks(&symlinks, config_root)?;

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

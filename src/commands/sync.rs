use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::config::load_workspace;
use crate::git::is_tree_modified;
use crate::hooks;
use crate::resolve::create_symlinks;

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
    // Every step runs, whatever an earlier one could not do for some entry:
    // one repository failing must not leave the rest unpulled, unlinked or
    // unpushed. The first failure is what sync exits with, at the end.
    let mut failed: Option<anyhow::Error> = None;
    let mut keep = |result: Result<()>| {
        if let Err(e) = result {
            failed.get_or_insert(e);
        }
    };
    // pull runs its own post_sync hook, skip it here to avoid double-run
    keep(crate::commands::pull::run_no_hooks(
        root,
        names,
        verbose,
        no_cache,
        interactive,
        out,
        err,
    ));

    // Restore symlinks for unlinked clones and remove orphaned links before
    // pushing, so local hygiene isn't blocked by a remote/auth failure.
    // A graph that does not resolve has no links to restore: pull already
    // failed with the reason. Offline: pull has just fetched what it reads.
    let resolved = crate::store::Sources::new(&config_root, no_cache).and_then(|sources| {
        crate::resolve::workspace(&config, &config_root, false, &sources, None, verbose)
    });
    match resolved {
        Ok(resolution) => keep(relink(
            &resolution,
            names,
            &config_root,
            &config.resolve.hoist_dir,
            force,
            out,
        )),
        Err(e) => keep(Err(e)),
    }

    // Only checkouts on the topic have anything to push; named, an implicit
    // one is pushed like any other.
    keep(crate::commands::push::run(
        root,
        names,
        interactive,
        out,
        err,
    ));

    // The hook is for a workspace that synced: not one left half done.
    if let Some(e) = failed {
        return Err(e);
    }
    hooks::run_post_sync(&config.hooks, &config_root, verbose, out)?;
    Ok(())
}

fn relink(
    resolution: &crate::resolution::Resolution,
    names: &[String],
    config_root: &Path,
    hoist_dir: &str,
    force: bool,
    out: &mut dyn Write,
) -> Result<()> {
    // Resolved against every repo, so dependencies are checked as a whole;
    // acted on only inside the repos named, since a link belongs to the repo
    // it sits in and that repo was not asked to sync.
    let repos = resolution.entries();
    let repos = repos.as_slice();
    let all_symlinks = &resolution.links;
    let selected = resolution.select(names)?;
    let symlinks: Vec<_> = all_symlinks
        .iter()
        .filter(|sym| {
            crate::resolve::owning_entry(&sym.link_path, repos)
                .is_some_and(|owner| selected.iter().any(|e| e.directory == owner.directory))
        })
        .cloned()
        .collect();

    // Checkouts gitscale made that nothing needs any more — an entry
    // removed or renamed, an implicit dependency nobody asks for — go, unless
    // that would lose something: then only with --force, like an unlinked
    // clone. An artefact holds nobody's work: every pull replaces it whole.
    let mut stale_skipped = 0usize;
    if names.is_empty() {
        for (dir, kind) in crate::ledger::left_behind(config_root, repos) {
            let stale = config_root.join(&dir);
            let git = kind == crate::ledger::Recorded::Git;
            if git && is_tree_modified(&stale) && !force {
                writeln!(
                    out,
                    "  skip  {} (no longer needed, but modified; use --force to remove)",
                    dir
                )?;
                stale_skipped += 1;
                continue;
            }
            crate::git::restore_writable(&stale)?;
            std::fs::remove_dir_all(&stale)?;
            if !git {
                crate::artefact::forget_install(config_root, &dir);
            }
            crate::ledger::forget(config_root, &dir)?;
            writeln!(out, "  remove  {} (no longer needed)", dir)?;
        }
        crate::ledger::prune(config_root)?;
    }

    // Remove orphaned symlinks (links whose dep was removed from config).
    // Broken orphans are always safe to remove; orphans that still resolve to a
    // valid checkout are only removed with --force.
    let orphans =
        crate::resolve::find_orphan_links(&selected, config_root, all_symlinks, hoist_dir);
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

    if skipped > 0 || orphan_skipped > 0 || stale_skipped > 0 {
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
        if stale_skipped > 0 {
            parts.push(format!(
                "{} checkout(s) no longer needed, with local modifications",
                stale_skipped
            ));
        }
        anyhow::bail!("{} (use --force to override)", parts.join(" and "));
    }

    Ok(())
}

//! `git scale sync`, and the placement every other command ends with: put
//! every checkout where resolution says, check out what is missing, relink,
//! prune images, and run `post_sync`.
//!
//! Typed, the hook and CI run it; `git scale pull` ends with it, and every
//! other forwarded command that moved a `HEAD` too.

use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::artefact::Artefacts;
use crate::checkout::Placer;
use crate::config::GitScaleConfig;
use crate::git::is_tree_modified;
use crate::hooks;
use crate::progress::{run_entries, RepoStatus};
use crate::resolve::{create_symlinks, is_outer_link, Network};
use crate::store::Sources;

/// What one placement does.
pub struct Placement<'a> {
    /// The slots to place, by directory: every one when empty.
    pub dirs: &'a [String],
    pub network: Network,
    /// Relink checkouts with work, remove orphan links whose target still
    /// resolves, and checkouts nothing needs that hold work.
    pub force: bool,
    /// A child left where git put it: a hook in that child runs placement
    /// for the rest of the workspace.
    pub leave: Option<&'a str>,
    /// The line before the result lines; none when empty.
    pub heading: &'a str,
}

/// `git scale sync [--force] [DIR...]`.
#[allow(clippy::too_many_arguments)]
pub fn run(
    start: Option<&Path>,
    dirs: &[String],
    verbose: bool,
    no_cache: bool,
    force: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = crate::config::load_workspace(start)?;
    let mut named = Vec::new();
    if !dirs.is_empty() {
        let here = crate::paths::Here::new(start, &config_root)?;
        let sources = Sources::new(&config_root, no_cache)?;
        // Offline: only to tell what the names mean.
        let resolution =
            crate::resolve::workspace(&config, &config_root, false, &sources, None, false)?;
        for dir in dirs {
            match here.name(&resolution, dir, true)? {
                // The root's own placement is every checkout's.
                crate::paths::Named::Root => {
                    named.clear();
                    break;
                }
                crate::paths::Named::Slot(dir) => named.push(dir),
            }
        }
    }
    place(
        &config,
        &config_root,
        &Placement {
            dirs: &named,
            network: Network::Online,
            force,
            leave: None,
            heading: "Pulling latest changes...",
        },
        verbose,
        no_cache,
        interactive,
        out,
        err,
    )
}

/// Place the workspace at `config_root`. Every step runs, whatever an
/// earlier one could not do for some entry: one repository failing must not
/// leave the rest unplaced or unlinked. The first failure is what it returns,
/// at the end; `post_sync` runs only when there was none.
#[allow(clippy::too_many_arguments)]
pub fn place(
    config: &GitScaleConfig,
    config_root: &Path,
    opts: &Placement,
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let mut failed: Option<anyhow::Error> = None;
    let mut keep = |result: Result<()>| {
        if let Err(e) = result {
            failed.get_or_insert(e);
        }
    };
    if config.repos.is_empty() {
        writeln!(out, "Nothing to sync.")?;
    }

    let sources = Sources::new(config_root, no_cache)?;
    if let Some(stores) = &sources.stores {
        // A removed root worktree keeps its branches checked out until its
        // entries go, and refuses them to every other worktree.
        stores.tidy(config_root);
    }
    // Branched from a topic a moment ago: the children come along.
    if let crate::topic::Root::Topic(branch) = crate::topic::root(config, config_root, true) {
        if let Some(from) = crate::topic::branched_from(config_root, &branch) {
            for carried in crate::checkout::carry(&sources, config_root, &from, &branch) {
                writeln!(out, "  carry  {} → {}", carried, branch)?;
            }
        }
    }
    let artefacts = Artefacts::new(config, config_root, sources.images());
    // Placed where a fresh clone would land: resolved against the remotes
    // as they are now, so a revision a dependency starts asking for in this
    // very placement is the one its checkout moves to.
    let resolution = crate::resolve::workspace_with(
        config,
        config_root,
        opts.network.in_ci(),
        &sources,
        Some(&artefacts),
        verbose,
    )?;
    let selected = resolution.select(opts.dirs)?;
    let placer = Placer {
        config_root,
        sources: &sources,
        artefacts: &artefacts,
        verbose,
        fetch: opts.network.in_ci() == Network::Online,
    };

    if !config.repos.is_empty() || !resolution.slots.is_empty() {
        keep(run_entries(
            opts.heading,
            "place",
            &selected,
            interactive,
            |entry| {
                let name = entry.directory.as_str();
                let dest = config_root.join(&entry.directory);
                if opts.leave == Some(name) {
                    return RepoStatus::Skip(format!("{} (left where git put it)", name));
                }
                if dest.is_symlink() {
                    // A link out of the workspace: whoever put it there owns
                    // that checkout.
                    if is_outer_link(&dest, config_root) {
                        return RepoStatus::Skip(format!("{} (symlink)", name));
                    }
                    if let Err(e) = std::fs::remove_file(&dest) {
                        return RepoStatus::Fail(format!(
                            "{}: failed to remove symlink: {}",
                            name, e
                        ));
                    }
                }
                let Some(slot) = resolution.slot(name) else {
                    return RepoStatus::Fail(format!("{}: not resolved", name));
                };
                match placer.place(slot) {
                    Ok(message) => RepoStatus::Ok(message),
                    Err(e) => RepoStatus::Fail(format!(
                        "{}: {:#}",
                        name,
                        crate::git::with_revision_hint(&entry.revision, e)
                    )),
                }
            },
            out,
            err,
        ));
    }

    keep(create_symlinks(&resolution.links, config_root));
    keep(crate::ledger::record(config_root, &resolution.entries()));
    keep(relink(
        &resolution,
        opts.dirs,
        config_root,
        &config.resolve.hoist_dir,
        opts.force,
        out,
    ));
    // What the placement put in the working trees, out of git's way.
    keep(crate::exclude::update(config_root, &resolution));

    // A placement updates tracked files in place, which is what keeps
    // unchanged files' mtimes — and so a build cache — valid across jobs.
    // The price is that anything untracked survives too, so in CI follow it
    // with exactly `git scale clean -fdx <each placed checkout>`. The root is
    // not named: in CI it is the runner's to clean, and a job that places
    // after restoring a build cache into it must not lose that cache.
    if crate::git::is_ci() && !selected.is_empty() {
        let dirs: Vec<String> = selected.iter().map(|e| e.directory.clone()).collect();
        keep(crate::commands::clean::scrub(
            config,
            config_root,
            &dirs,
            interactive,
            out,
            err,
        ));
    }

    // Images nothing has used for a while, at most once a day: a stat when
    // there is nothing to do.
    if let Some(stores) = &sources.stores {
        let pruned = crate::store::keep_recent(config.clean.keep_recent.as_deref())
            .and_then(|keep| stores.prune_images_daily(keep));
        match pruned {
            Ok(Some(pruned)) if verbose && pruned.images + pruned.entries > 0 => writeln!(
                out,
                "  pruned {} ({} freed)",
                crate::cache::plural(pruned.images + pruned.entries, "image", "images"),
                crate::cache::human_size(pruned.freed)
            )?,
            Ok(_) => {}
            Err(e) => keep(Err(e)),
        }
    }

    // The hook is for a workspace that was placed: not one left half done.
    if let Some(e) = failed {
        return Err(e);
    }
    hooks::run_post_sync(&config.hooks, config_root, verbose, out)
}

/// Restore dedup links a real checkout replaced, remove checkouts nothing
/// needs, and remove orphan links. Anything holding work is reported and
/// kept unless `force`, and fails the placement.
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
    // A CI checkout holds nobody's work: what an earlier job left is never a
    // reason to keep it.
    let force = force || crate::git::is_ci();
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
    let color = crate::output::stdout();
    let skip = |out: &mut dyn Write, msg: String| -> std::io::Result<()> {
        writeln!(
            out,
            "{}",
            crate::output::result_line(crate::output::Outcome::Skip, &msg, color)
        )
    };

    // Checkouts gitscale made that nothing needs any more — an entry
    // removed or renamed, an implicit dependency nobody asks for — go, unless
    // that would lose something: then only with --force, like an unlinked
    // checkout. An artefact holds nobody's work: every placement replaces it
    // whole.
    let mut stale_skipped = 0usize;
    if names.is_empty() {
        for (dir, kind) in crate::ledger::left_behind(config_root, repos) {
            let stale = config_root.join(&dir);
            let git = kind == crate::ledger::Recorded::Git;
            if git && is_tree_modified(&stale) && !force {
                skip(
                    out,
                    format!(
                        "{} (no longer needed, but modified; use --force to remove)",
                        dir
                    ),
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

    // Orphan links: links whose dependency left every config. A broken one
    // is always safe to remove; one that still resolves only with --force.
    let orphans =
        crate::resolve::find_orphan_links(&selected, config_root, all_symlinks, hoist_dir);
    let mut orphan_skipped = 0usize;
    for orphan in &orphans {
        let link_abs = config_root.join(&orphan.link_path);
        if orphan.broken || force {
            std::fs::remove_file(&link_abs)?;
            writeln!(out, "  unlink  {} (orphan)", orphan.link_path.display())?;
        } else {
            skip(
                out,
                format!(
                    "{} (orphan with valid target, use --force to remove)",
                    orphan.link_path.display()
                ),
            )?;
            orphan_skipped += 1;
        }
    }

    // Unlinked checkouts: a real directory where a dedup link belongs.
    let mut skipped = 0usize;
    for sym in &symlinks {
        let link_abs = config_root.join(&sym.link_path);
        if !link_abs.exists()
            || link_abs
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        {
            continue;
        }
        if is_tree_modified(&link_abs) && !force {
            skip(
                out,
                format!(
                    "{} (modified, use --force to relink)",
                    sym.link_path.display()
                ),
            )?;
            skipped += 1;
            continue;
        }
        // Removed, for create_symlinks to put the link back.
        crate::git::restore_writable(&link_abs)?;
        std::fs::remove_dir_all(&link_abs)?;
        writeln!(out, "  relink  {}", sym.link_path.display())?;
    }
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

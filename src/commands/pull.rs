use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::artefact::Artefacts;
use crate::checkout::Placer;
use crate::config::load_workspace;
use crate::hooks;
use crate::progress::{run_entries, RepoStatus};
use crate::resolve::is_outer_link;
use crate::store::Sources;

pub fn run(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) =
        pull_inner(root, names, verbose, no_cache, interactive, true, out, err)?;
    hooks::run_post_sync(&config.hooks, &config_root, verbose, out)?;
    Ok(())
}

/// Pull without running hooks — used by sync to avoid double-running.
pub fn run_no_hooks(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    // Not scrubbed: sync decides for itself what untracked content goes —
    // an orphan it would keep without --force must not disappear here first.
    pull_inner(root, names, verbose, no_cache, interactive, false, out, err)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn pull_inner(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    scrub: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(crate::config::GitScaleConfig, std::path::PathBuf)> {
    let (config, config_root) = load_workspace(root)?;
    if config.repos.is_empty() {
        writeln!(out, "Nothing to pull.")?;
        return Ok((config, config_root));
    }

    let sources = Sources::new(&config_root, no_cache)?;
    if let Some(stores) = &sources.stores {
        // A removed root worktree keeps its branches checked out until its
        // entries go, and refuses them to every other worktree.
        stores.tidy(&config_root);
    }
    // Branched from a topic a moment ago: the children come along.
    if let crate::topic::Root::Topic(branch) = crate::topic::root(&config, &config_root, true) {
        if let Some(from) = crate::topic::branched_from(&config_root, &branch) {
            for carried in crate::checkout::carry(&sources, &config_root, &from, &branch) {
                writeln!(out, "  carry  {} → {}", carried, branch)?;
            }
        }
    }
    let artefacts = Artefacts::new(&config, &config_root, sources.images());
    // Pull to where a fresh clone would land: resolved against the remote
    // as it is now, so a revision a dependency starts asking for in this very
    // pull is the one its checkout moves to.
    let resolution = crate::resolve::workspace(
        &config,
        &config_root,
        true,
        &sources,
        Some(&artefacts),
        verbose,
    )?;
    let selected = resolution.select(names)?;
    let placer = Placer {
        config_root: &config_root,
        sources: &sources,
        artefacts: &artefacts,
        verbose,
    };

    // A failed entry fails the command, but only after the rest is done:
    // links planted, and in CI every checkout that did move scrubbed.
    let pulled = run_entries(
        "Pulling latest changes...",
        "pull",
        &selected,
        interactive,
        |entry| {
            let name = entry.directory.as_str();
            let dest = config_root.join(&entry.directory);

            if dest.is_symlink() {
                // An enclosing workspace's dedup link: its root decides that
                // checkout's revision, and pulling through the link would move
                // it to this config's instead. Named, it is unlinked.
                if names.is_empty() && is_outer_link(&dest, &config_root) {
                    return RepoStatus::Skip(format!("{} (symlink)", name));
                }
                if let Err(e) = std::fs::remove_file(&dest) {
                    return RepoStatus::Fail(format!("{}: failed to remove symlink: {}", name, e));
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
    );

    crate::resolve::create_symlinks(&resolution.links, &config_root)?;
    crate::ledger::record(&config_root, &resolution.entries())?;

    // A pull updates tracked files in place, which is what keeps unchanged
    // files' mtimes — and so a build cache — valid across jobs. The price is
    // that anything untracked survives too, so in CI follow it with exactly
    // `gitscale clean -f <each pulled checkout>`. The root is not named: in CI
    // it is the runner's to clean, and a job that pulls after restoring a build
    // cache into it must not lose that cache.
    if crate::git::is_ci() && scrub {
        let dir_names: Vec<String> = selected.iter().map(|e| e.directory.clone()).collect();
        crate::commands::clean::run(
            Some(&config_root),
            &dir_names,
            &[],
            false,
            None,
            true,
            interactive,
            out,
            err,
        )?;
    }

    // Images nothing has used for a while, at most once a day: a stat when
    // there is nothing to do.
    if let Some(stores) = &sources.stores {
        let keep = crate::store::keep_recent(config.clean.keep_recent.as_deref())?;
        if let Some(pruned) = stores.prune_images_daily(keep)? {
            if verbose && pruned.images + pruned.entries > 0 {
                writeln!(
                    out,
                    "  pruned {} ({} freed)",
                    crate::cache::plural(pruned.images + pruned.entries, "image", "images"),
                    crate::cache::human_size(pruned.freed)
                )?;
            }
        }
    }

    pulled?;
    Ok((config, config_root))
}

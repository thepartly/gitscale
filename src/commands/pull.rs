use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::artefact::{Artefacts, Pulled};
use crate::commands::cache::Sources;
use crate::config::load_workspace;
use crate::git::pull_repo;
use crate::hooks;
use crate::progress::{run_entries, RepoStatus};
use crate::resolve::is_outer_link;

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

    // A hook-triggered pull is the first thing to run in a new worktree, so
    // this is the path that populates it — and the one that benefits most.
    let sources = Sources::adopting(&config, &config_root, no_cache, verbose, out)?;
    let artefacts = Artefacts::new(&config, &config_root, sources.cache.clone());
    // Pull to where a fresh clone would land: resolved against the remote
    // as it is now, so a revision a dependency starts asking for in this very
    // pull is the one its checkout moves to.
    let resolution = crate::resolve::workspace(
        &config,
        &config_root,
        true,
        sources.cache.clone(),
        Some(&artefacts),
        verbose,
    )?;
    let selected = resolution.select(names)?;

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
                // it to this config's instead. Named, it is unlinked, as
                // `clone` would.
                if names.is_empty() && is_outer_link(&dest, &config_root) {
                    return RepoStatus::Skip(format!("{} (symlink)", name));
                }
                // Anything else is replaced by a real clone, as `clone` does.
                if let Err(e) = std::fs::remove_file(&dest) {
                    return RepoStatus::Fail(format!("{}: failed to remove symlink: {}", name, e));
                }
            }

            if entry.is_artefact() {
                return match artefacts.pull(entry, &dest) {
                    Ok(Pulled::Updated(commit)) => RepoStatus::Ok(format!(
                        "{} (artefact {})",
                        name,
                        crate::git::short_sha(&commit)
                    )),
                    Ok(Pulled::Current(commit)) => RepoStatus::Ok(format!(
                        "{} (artefact {}, up to date)",
                        name,
                        crate::git::short_sha(&commit)
                    )),
                    Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
                };
            }

            // Cache first: the entry is updated from the remote, then the
            // workspace is updated from the entry. N workspaces share step one,
            // which is the whole saving.
            let from = sources.for_entry(entry);
            match pull_repo(entry, &config_root, verbose, &from) {
                Ok(()) => RepoStatus::Ok(name.to_string()),
                Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
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
    if sources.ci && scrub {
        let dir_names: Vec<String> = selected.iter().map(|e| e.directory.clone()).collect();
        crate::commands::clean::run(
            Some(&config_root),
            &dir_names,
            &[],
            true,
            interactive,
            out,
            err,
        )?;
    }

    pulled?;
    Ok((config, config_root))
}

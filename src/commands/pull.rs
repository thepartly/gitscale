use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use crate::artefact::{Artefacts, Pulled};
use crate::commands::cache::Sources;
use crate::commands::clone::move_to_adopted;
use crate::config::{filter_entries, load_workspace, RepoEntry};
use crate::git::pull_repo;
use crate::hooks;
use crate::progress::{run_entries, RepoStatus};
use crate::resolve::{is_outer_link, resolve_recursive};

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
    let selected = filter_entries(&config.repos, names)?;

    if selected.is_empty() {
        writeln!(out, "Nothing to pull.")?;
        return Ok((config, config_root));
    }

    // Pull to where a fresh clone would land, which for an entry with no
    // revision of its own is the one a child pins. Read from the child
    // configs as they are now; one this pull changes is caught after it.
    // Unresolvable here (a stale child, say) means pulling as declared: the
    // resolution after the pull reports whatever is still wrong.
    let adopted_before: HashMap<String, String> = resolve_recursive(&config.repos, &config_root)
        .map(|(_, adopted)| adopted.into_iter().collect())
        .unwrap_or_default();
    let effective: Vec<RepoEntry> = selected
        .iter()
        .map(|e| match adopted_before.get(&e.directory) {
            Some(revision) if e.revision.is_empty() => RepoEntry {
                revision: revision.clone(),
                ..(*e).clone()
            },
            _ => (*e).clone(),
        })
        .collect();
    // A hook-triggered pull is the first thing to run in a new worktree, so
    // this is the path that populates it — and the one that benefits most.
    let sources = Sources::adopting(&config, &config_root, no_cache, verbose, out)?;
    let artefacts = Artefacts::new(&config, &config_root, sources.cache.clone());

    run_entries(
        "Pulling latest changes...",
        "pull",
        &effective,
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
    )?;

    // Re-resolve symlinks after pull (child configs may have changed), and
    // move any selected entry whose adopted revision the pull itself changed.
    let adopted = crate::resolve::resolve_and_link(&config.repos, &config_root)?;
    let changed = adopted.into_iter().filter(|(directory, revision)| {
        selected.iter().any(|e| e.directory == *directory)
            && adopted_before.get(directory) != Some(revision)
    });
    move_to_adopted(&config.repos, &config_root, changed, &sources, verbose)?;

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

    Ok((config, config_root))
}

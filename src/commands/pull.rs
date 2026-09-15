use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use crate::cache;
use crate::commands::cache as cache_cmd;
use crate::commands::clone::filter_entries;
use crate::config::{find_config, load_config};
use crate::git::{is_ci, pull_repo};
use crate::hooks;
use crate::progress::{run_parallel, RepoStatus};
use crate::share;
use crate::storage::pull_artefact;

pub fn run(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = pull_inner(root, names, verbose, no_cache, interactive, out, err)?;
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
    pull_inner(root, names, verbose, no_cache, interactive, out, err)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn pull_inner(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(crate::config::GitScaleConfig, std::path::PathBuf)> {
    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;
    let selected = filter_entries(&config.repos, names)?;

    if selected.is_empty() {
        writeln!(out, "Nothing to pull.")?;
        return Ok((config, config_root));
    }

    let entry_map: HashMap<&str, &crate::config::RepoEntry> =
        selected.iter().map(|e| (e.directory.as_str(), e)).collect();
    let dir_names: Vec<String> = selected.iter().map(|e| e.directory.clone()).collect();
    let storage_url = &config.storage_url;
    let ci = is_ci();
    // A hook-triggered pull is the first thing to run in a new worktree, so
    // this is the path that populates it — and the one that benefits most.
    let source = share::source_workspace(&config_root);
    let dissociate = config.share.dissociate;
    let cache = cache_cmd::open(&config, no_cache);
    cache_cmd::adopt_root(cache.as_ref(), &config, &config_root, out)?;

    let failed = run_parallel(
        "Pulling latest changes...",
        &dir_names,
        interactive,
        |name| {
            let entry = &entry_map[name];

            if entry.is_artefact() {
                if storage_url.is_empty() {
                    return RepoStatus::Fail(format!("{}: no [storage] configured", name));
                }
                let dest = config_root.join(&entry.directory);
                let revision = if entry.revision.is_empty() {
                    "HEAD"
                } else {
                    &entry.revision
                };
                return match pull_artefact(storage_url, &entry.repo_url, revision, &dest) {
                    Ok(true) => RepoStatus::Ok(format!("{} (artefact)", name)),
                    Ok(false) => RepoStatus::Skip(format!("{} (no remote artefact)", name)),
                    Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
                };
            }

            // Cache first: the entry is updated from the remote, then the
            // workspace is updated from the entry. N workspaces share step one,
            // which is the whole saving.
            let from = cache::source_for(
                cache.as_ref(),
                source.as_deref(),
                entry,
                dissociate,
                ci,
                verbose,
            );
            match pull_repo(entry, &config_root, verbose, &from) {
                Ok(()) => RepoStatus::Ok(name.to_string()),
                Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
            }
        },
        out,
        err,
    )?;

    if failed > 0 {
        anyhow::bail!("{} repo(s) failed to pull", failed);
    }

    // Re-resolve symlinks after pull (child configs may have changed)
    crate::resolve::resolve_and_link(&config.repos, &config_root, false, out)?;

    Ok((config, config_root))
}

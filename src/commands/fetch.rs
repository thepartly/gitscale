use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::artefact::Artefacts;
use crate::commands::cache::Sources;
use crate::config::{filter_entries, load_workspace};
use crate::git::fetch_repo;
use crate::progress::{run_entries, RepoStatus};

pub fn run(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    if config.repos.is_empty() {
        writeln!(out, "Nothing to fetch.")?;
        return Ok(());
    }

    let sources = Sources::adopting(&config, &config_root, no_cache, verbose, out)?;
    let artefacts = Artefacts::new(&config, &config_root, sources.cache.clone());
    // Refreshes what resolution reads, so the next offline `status` sees what
    // the remotes have now; and covers the checkouts nobody declared. A fetch
    // changes no checkout, so a graph that will not resolve — a remote that
    // cannot be reached, a conflict — still gets the declared entries fetched,
    // and the command fails afterwards with the reason.
    let resolved = crate::resolve::workspace(
        &config,
        &config_root,
        true,
        sources.cache.clone(),
        Some(&artefacts),
        verbose,
    );
    let (selected, unresolved) = match resolved {
        Ok(resolution) => (resolution.select(names)?, None),
        Err(e) => (filter_entries(&config.repos, names)?, Some(e)),
    };

    let fetched = run_entries(
        "Fetching...",
        "fetch",
        &selected,
        interactive,
        |entry| {
            let name = entry.directory.as_str();

            if entry.is_artefact() {
                let dest = config_root.join(&entry.directory);
                if dest.is_symlink() {
                    return RepoStatus::Skip(format!("{} (symlink)", name));
                }
                return match artefacts.fetch(entry) {
                    Ok(commit) => RepoStatus::Ok(format!(
                        "{} (artefact {})",
                        name,
                        crate::git::short_sha(&commit)
                    )),
                    Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
                };
            }

            let dest = config_root.join(&entry.directory);
            if !crate::git::is_checkout(&dest) {
                return RepoStatus::Skip(format!("{} (not cloned)", name));
            }
            // A symlinked entry is another entry's checkout, handled under
            // that entry's own name and revision.
            if dest.is_symlink() {
                return RepoStatus::Skip(format!("{} (symlink)", name));
            }

            // Updates the cache entry as well: a fetch that only advanced
            // this workspace's refs would leave every other one to download
            // the same objects again.
            let from = sources.for_entry(entry);
            match fetch_repo(entry, &config_root, &from) {
                Ok(()) => RepoStatus::Ok(name.to_string()),
                Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
            }
        },
        out,
        err,
    );
    // The reasons in the message itself: the CLI prints only the outermost
    // error of a chain, and neither failure may hide the other.
    match (fetched, unresolved) {
        (fetched, None) => fetched,
        (Ok(()), Some(e)) => Err(anyhow::anyhow!(
            "cannot resolve the workspace's dependencies: {:#}",
            e
        )),
        (Err(failed), Some(e)) => Err(anyhow::anyhow!(
            "{:#}; and cannot resolve the workspace's dependencies: {:#}",
            failed,
            e
        )),
    }
}

use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::commands::cache::Sources;
use crate::config::{filter_entries, load_workspace};
use crate::git::fetch_repo;
use crate::progress::{run_entries, RepoStatus};
use crate::storage::fetch_artefact;

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
    let selected = filter_entries(&config.repos, names)?;

    if selected.is_empty() {
        writeln!(out, "Nothing to fetch.")?;
        return Ok(());
    }

    let storage_url = &config.storage_url;
    let sources = Sources::adopting(&config, &config_root, no_cache, verbose, out)?;

    run_entries(
        "Fetching...",
        "fetch",
        &selected,
        interactive,
        |entry| {
            let name = entry.directory.as_str();

            if entry.is_artefact() {
                if storage_url.is_empty() {
                    return RepoStatus::Fail(format!("{}: no [storage] configured", name));
                }
                let dest = config_root.join(&entry.directory);
                return match fetch_artefact(storage_url, &entry.repo_url, &entry.revision, &dest) {
                    Ok(result) => {
                        if result.exists {
                            RepoStatus::Ok(format!("{} (artefact)", name))
                        } else {
                            RepoStatus::Skip(format!("{} (no remote artefact)", name))
                        }
                    }
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
    )
}

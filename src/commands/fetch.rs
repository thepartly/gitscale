use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use crate::commands::clone::filter_entries;
use crate::config::{find_config, load_config};
use crate::git::fetch_repo;
use crate::progress::{run_parallel, RepoStatus};
use crate::storage::fetch_artefact;

pub fn run(
    root: Option<&Path>,
    names: &[String],
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;
    let selected = filter_entries(&config.repos, names)?;

    if selected.is_empty() {
        writeln!(out, "Nothing to fetch.")?;
        return Ok(());
    }

    let entry_map: HashMap<&str, &crate::config::RepoEntry> = selected
        .iter()
        .map(|e| (e.directory.as_str(), e))
        .collect();
    let dir_names: Vec<String> = selected.iter().map(|e| e.directory.clone()).collect();
    let storage_url = &config.storage_url;

    let failed = run_parallel(
        "Fetching...",
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
                return match fetch_artefact(storage_url, &entry.repo_url, revision, &dest) {
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
            if !dest.exists() {
                return RepoStatus::Skip(format!("{} (not cloned)", name));
            }

            match fetch_repo(entry, &config_root) {
                Ok(()) => RepoStatus::Ok(name.to_string()),
                Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
            }
        },
        out,
        err,
    )?;

    if failed > 0 {
        anyhow::bail!("{} repo(s) failed to fetch", failed);
    }
    Ok(())
}

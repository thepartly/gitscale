use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use crate::config::{find_config, load_config, RepoEntry};
use crate::git::{clone_repo, is_ci};
use crate::progress::{run_parallel, RepoStatus};
use crate::storage::clone_artefact;

pub fn run(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;
    let selected = filter_entries(&config.repos, names)?;

    if selected.is_empty() {
        writeln!(out, "Nothing to clone.")?;
        return Ok(());
    }

    let entry_map: HashMap<&str, &RepoEntry> =
        selected.iter().map(|e| (e.directory.as_str(), e)).collect();
    let dir_names: Vec<String> = selected.iter().map(|e| e.directory.clone()).collect();
    let storage_url = &config.storage_url;
    let ci = is_ci();

    let failed = run_parallel(
        "Cloning missing repos...",
        &dir_names,
        interactive,
        |name| {
            let entry = &entry_map[name];
            let dest = config_root.join(&entry.directory);

            if entry.is_artefact() {
                if dest.exists() {
                    return RepoStatus::Skip(format!("{} (already exists)", name));
                }
                if storage_url.is_empty() {
                    return RepoStatus::Fail(format!("{}: no [storage] configured", name));
                }
                let revision = if entry.revision.is_empty() {
                    "HEAD"
                } else {
                    &entry.revision
                };
                return match clone_artefact(storage_url, &entry.repo_url, revision, &dest) {
                    Ok(true) => RepoStatus::Ok(format!("{} (artefact)", name)),
                    Ok(false) => RepoStatus::Skip(format!("{} (no artefact data)", name)),
                    Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
                };
            }

            if dest.exists() {
                return RepoStatus::Skip(format!("{} (already exists)", name));
            }

            let shallow = ci || entry.is_readonly();
            match clone_repo(entry, &config_root, verbose, shallow) {
                Ok(()) => RepoStatus::Ok(name.to_string()),
                Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
            }
        },
        out,
        err,
    )?;

    if failed > 0 {
        anyhow::bail!("{} repo(s) failed to clone", failed);
    }

    crate::resolve::resolve_and_link(&config.repos, &config_root, true, out)?;

    Ok(())
}

pub fn filter_entries(entries: &[RepoEntry], names: &[String]) -> Result<Vec<RepoEntry>> {
    if names.is_empty() {
        return Ok(entries.to_vec());
    }
    let matched: Vec<RepoEntry> = entries
        .iter()
        .filter(|e| names.contains(&e.directory))
        .cloned()
        .collect();
    let matched_names: Vec<&str> = matched.iter().map(|e| e.directory.as_str()).collect();
    let unknown: Vec<&str> = names
        .iter()
        .filter(|n| !matched_names.contains(&n.as_str()))
        .map(|n| n.as_str())
        .collect();
    if !unknown.is_empty() {
        anyhow::bail!("Unknown repos: {}", unknown.join(", "));
    }
    Ok(matched)
}

use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::config::{find_config, load_config, RepoEntry};
use crate::git::{clone_repo, is_ci};
use crate::storage::clone_artefact;

pub fn run(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
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

    let mut failed = 0;
    for entry in &selected {
        let dest = config_root.join(&entry.directory);
        if entry.is_artefact() {
            if dest.exists() {
                writeln!(out, "  skip  {} (already exists)", entry.directory)?;
                continue;
            }
            if config.storage_url.is_empty() {
                writeln!(err, "  FAIL  {}: no [storage] configured", entry.directory)?;
                failed += 1;
                continue;
            }
            let revision = if entry.revision.is_empty() {
                "HEAD"
            } else {
                &entry.revision
            };
            match clone_artefact(&config.storage_url, &entry.repo_url, revision, &dest) {
                Ok(true) => writeln!(out, "  ok    {} (artefact)", entry.directory)?,
                Ok(false) => writeln!(out, "  skip  {} (no artefact data)", entry.directory)?,
                Err(e) => {
                    writeln!(err, "  FAIL  {}: {}", entry.directory, e)?;
                    failed += 1;
                }
            }
            continue;
        }
        if dest.exists() {
            writeln!(out, "  skip  {} (already exists)", entry.directory)?;
            continue;
        }
        if verbose {
            writeln!(
                out,
                "  clone {} → {} @ {}",
                entry.repo_url, entry.directory, entry.revision
            )?;
        }
        let ci = is_ci();
        let shallow = ci || entry.is_readonly();
        match clone_repo(entry, &config_root, verbose, shallow) {
            Ok(()) => writeln!(out, "  ok    {}", entry.directory)?,
            Err(e) => {
                writeln!(err, "  FAIL  {}: {}", entry.directory, e)?;
                failed += 1;
            }
        }
    }

    if failed > 0 {
        anyhow::bail!("{} repo(s) failed to clone", failed);
    }
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

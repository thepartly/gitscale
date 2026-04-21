use anyhow::Result;
use std::path::Path;

use crate::config::{find_config, load_config, RepoEntry};
use crate::git::{clone_repo, is_ci};
use crate::storage::clone_artefact;

pub fn run(root: Option<&Path>, names: &[String], verbose: bool) -> Result<()> {
    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;
    let selected = filter_entries(&config.repos, names)?;

    if selected.is_empty() {
        println!("Nothing to clone.");
        return Ok(());
    }

    let mut failed = 0;
    for entry in &selected {
        let dest = config_root.join(&entry.directory);
        if entry.is_artefact() {
            if dest.exists() {
                println!("  skip  {} (already exists)", entry.directory);
                continue;
            }
            if config.storage_url.is_empty() {
                eprintln!("  FAIL  {}: no [storage] configured", entry.directory);
                failed += 1;
                continue;
            }
            let revision = if entry.revision.is_empty() {
                "HEAD"
            } else {
                &entry.revision
            };
            match clone_artefact(&config.storage_url, &entry.repo_url, revision, &dest) {
                Ok(true) => println!("  ok    {} (artefact)", entry.directory),
                Ok(false) => println!("  skip  {} (no artefact data)", entry.directory),
                Err(e) => {
                    eprintln!("  FAIL  {}: {}", entry.directory, e);
                    failed += 1;
                }
            }
            continue;
        }
        if dest.exists() {
            println!("  skip  {} (already exists)", entry.directory);
            continue;
        }
        if verbose {
            println!(
                "  clone {} → {} @ {}",
                entry.repo_url, entry.directory, entry.revision
            );
        }
        let ci = is_ci();
        let shallow = ci || entry.is_readonly();
        match clone_repo(entry, &config_root, verbose, shallow) {
            Ok(()) => println!("  ok    {}", entry.directory),
            Err(e) => {
                eprintln!("  FAIL  {}: {}", entry.directory, e);
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

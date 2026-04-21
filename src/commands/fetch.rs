use anyhow::Result;
use std::path::Path;

use crate::commands::clone::filter_entries;
use crate::config::{find_config, load_config};
use crate::git::fetch_repo;
use crate::storage::fetch_artefact;

pub fn run(root: Option<&Path>, names: &[String], verbose: bool) -> Result<()> {
    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;
    let selected = filter_entries(&config.repos, names)?;

    if selected.is_empty() {
        println!("Nothing to fetch.");
        return Ok(());
    }

    let mut failed = 0;
    for entry in &selected {
        if entry.is_artefact() {
            if config.storage_url.is_empty() {
                eprintln!("  FAIL  {}: no [storage] configured", entry.directory);
                failed += 1;
                continue;
            }
            let dest = config_root.join(&entry.directory);
            let revision = if entry.revision.is_empty() {
                "HEAD"
            } else {
                &entry.revision
            };
            match fetch_artefact(&config.storage_url, &entry.repo_url, revision, &dest) {
                Ok(result) => {
                    if result.exists {
                        println!("  ok    {} (artefact)", entry.directory);
                    } else {
                        println!("  skip  {} (no remote artefact)", entry.directory);
                    }
                }
                Err(e) => {
                    eprintln!("  FAIL  {}: {}", entry.directory, e);
                    failed += 1;
                }
            }
            continue;
        }

        let dest = config_root.join(&entry.directory);
        if !dest.exists() {
            println!("  skip  {} (not cloned)", entry.directory);
            continue;
        }
        if verbose {
            println!("  fetch {}", entry.directory);
        }
        match fetch_repo(entry, &config_root) {
            Ok(()) => println!("  ok    {}", entry.directory),
            Err(e) => {
                eprintln!("  FAIL  {}: {}", entry.directory, e);
                failed += 1;
            }
        }
    }

    if failed > 0 {
        anyhow::bail!("{} repo(s) failed to fetch", failed);
    }
    Ok(())
}

use anyhow::Result;
use std::path::Path;

use crate::commands::clone::filter_entries;
use crate::config::{find_config, load_config};
use crate::git::{is_ci, pull_repo};
use crate::hooks;
use crate::storage::pull_artefact;

pub fn run(root: Option<&Path>, names: &[String], verbose: bool) -> Result<()> {
    let (config, config_root) = pull_inner(root, names, verbose)?;
    hooks::run_post_sync(&config.hooks, &config_root, verbose)?;
    Ok(())
}

/// Pull without running hooks — used by sync to avoid double-running.
pub fn run_no_hooks(root: Option<&Path>, names: &[String], verbose: bool) -> Result<()> {
    pull_inner(root, names, verbose)?;
    Ok(())
}

fn pull_inner(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
) -> Result<(crate::config::GitScaleConfig, std::path::PathBuf)> {
    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;
    let selected = filter_entries(&config.repos, names)?;

    if selected.is_empty() {
        println!("Nothing to pull.");
        return Ok((config, config_root));
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
            match pull_artefact(&config.storage_url, &entry.repo_url, revision, &dest) {
                Ok(true) => println!("  ok    {} (artefact)", entry.directory),
                Ok(false) => println!("  skip  {} (no remote artefact)", entry.directory),
                Err(e) => {
                    eprintln!("  FAIL  {}: {}", entry.directory, e);
                    failed += 1;
                }
            }
            continue;
        }

        if verbose {
            println!("  pull  {}", entry.directory);
        }
        let ci = is_ci();
        let shallow = ci || entry.is_readonly();
        match pull_repo(entry, &config_root, verbose, shallow) {
            Ok(()) => println!("  ok    {}", entry.directory),
            Err(e) => {
                eprintln!("  FAIL  {}: {}", entry.directory, e);
                failed += 1;
            }
        }
    }

    if failed > 0 {
        anyhow::bail!("{} repo(s) failed to pull", failed);
    }

    Ok((config, config_root))
}

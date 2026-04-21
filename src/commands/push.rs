use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::commands::clone::filter_entries;
use crate::config::{find_config, load_config};
use crate::git::push_repo;

pub fn run(root: Option<&Path>, names: &[String], verbose: bool, out: &mut dyn Write, err: &mut dyn Write) -> Result<()> {
    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;
    let selected = filter_entries(&config.repos, names)?;

    if selected.is_empty() {
        writeln!(out, "Nothing to push.")?;
        return Ok(());
    }

    let mut failed = 0;
    for entry in &selected {
        if entry.is_artefact() {
            writeln!(out, "  skip  {} (artefact)", entry.directory)?;
            continue;
        }
        if entry.is_readonly() {
            writeln!(out, "  skip  {} (readonly)", entry.directory)?;
            continue;
        }
        let dest = config_root.join(&entry.directory);
        if !dest.exists() {
            writeln!(out, "  skip  {} (not cloned)", entry.directory)?;
            continue;
        }
        if verbose {
            writeln!(out, "  push  {}", entry.directory)?;
        }
        match push_repo(entry, &config_root, verbose) {
            Ok(()) => writeln!(out, "  ok    {}", entry.directory)?,
            Err(e) => {
                writeln!(err, "  FAIL  {}: {}", entry.directory, e)?;
                failed += 1;
            }
        }
    }

    if failed > 0 {
        anyhow::bail!("{} repo(s) failed to push", failed);
    }
    Ok(())
}

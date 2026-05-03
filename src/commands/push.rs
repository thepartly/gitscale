use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use crate::commands::clone::filter_entries;
use crate::config::{find_config, load_config};
use crate::git::push_repo;
use crate::progress::{run_parallel, RepoStatus};

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
        writeln!(out, "Nothing to push.")?;
        return Ok(());
    }

    let entry_map: HashMap<&str, &crate::config::RepoEntry> =
        selected.iter().map(|e| (e.directory.as_str(), e)).collect();
    let dir_names: Vec<String> = selected.iter().map(|e| e.directory.clone()).collect();

    let failed = run_parallel(
        "Pushing local changes...",
        &dir_names,
        interactive,
        |name| {
            let entry = &entry_map[name];

            if entry.is_artefact() {
                return RepoStatus::Skip(format!("{} (artefact)", name));
            }
            if entry.is_readonly() {
                return RepoStatus::Skip(format!("{} (readonly)", name));
            }
            let dest = config_root.join(&entry.directory);
            if !dest.exists() {
                return RepoStatus::Skip(format!("{} (not cloned)", name));
            }

            match push_repo(entry, &config_root, verbose) {
                Ok(()) => RepoStatus::Ok(name.to_string()),
                Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
            }
        },
        out,
        err,
    )?;

    if failed > 0 {
        anyhow::bail!("{} repo(s) failed to push", failed);
    }
    Ok(())
}

use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use crate::commands::clone::filter_entries;
use crate::config::{find_config, load_config};
use crate::git::{commit_path, is_repo_root};
use crate::progress::{run_parallel, RepoStatus};

pub fn run(
    root: Option<&Path>,
    names: &[String],
    message: &str,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    if message.trim().is_empty() {
        anyhow::bail!("commit message must not be empty");
    }

    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;
    let selected = filter_entries(&config.repos, names)?;

    let entry_map: HashMap<&str, &crate::config::RepoEntry> =
        selected.iter().map(|e| (e.directory.as_str(), e)).collect();
    let dir_names: Vec<String> = selected.iter().map(|e| e.directory.clone()).collect();

    let mut failed = run_parallel(
        "Committing local changes...",
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
            // Resolved recursive deps are symlinks to a root-level checkout that
            // is committed on its own; don't commit through the symlink.
            if dest.is_symlink() {
                return RepoStatus::Skip(format!("{} (symlink)", name));
            }

            match commit_path(&dest, message) {
                Ok(true) => RepoStatus::Ok(name.to_string()),
                Ok(false) => RepoStatus::Skip(format!("{} (clean)", name)),
                Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
            }
        },
        out,
        err,
    )?;

    // When committing everything, also commit the workspace repo itself, but
    // only if the config root is that repo's top level (never an ancestor repo).
    if names.is_empty() && is_repo_root(&config_root) {
        match commit_path(&config_root, message) {
            Ok(true) => writeln!(out, "  ok    . (workspace root)")?,
            Ok(false) => writeln!(out, "  skip  . (workspace root, clean)")?,
            Err(e) => {
                writeln!(out, "  FAIL  . (workspace root): {}", e)?;
                failed += 1;
            }
        }
    }

    if failed > 0 {
        anyhow::bail!("{} repo(s) failed to commit", failed);
    }
    Ok(())
}

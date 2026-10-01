use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use crate::commands::clone::filter_entries;
use crate::config::{find_config, load_config};
use crate::git::{is_detached, push_repo};
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
            if !crate::git::is_checkout(&dest) {
                return RepoStatus::Skip(format!("{} (not cloned)", name));
            }
            // A symlinked entry is another entry's checkout, handled under
            // that entry's own name and revision.
            if dest.is_symlink() {
                return RepoStatus::Skip(format!("{} (symlink)", name));
            }
            // A tag or SHA pin checks out detached: there is no branch to
            // push, and `git push` would fail the whole sync over it.
            if is_detached(entry, &config_root) {
                return RepoStatus::Skip(format!("{} (detached HEAD)", name));
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

use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::config::{filter_entries, load_workspace};
use crate::git::{is_detached, push_repo};
use crate::progress::{run_entries, RepoStatus};

pub fn run(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    let selected = filter_entries(&config.repos, names)?;

    if selected.is_empty() {
        writeln!(out, "Nothing to push.")?;
        return Ok(());
    }

    run_entries(
        "Pushing local changes...",
        "push",
        &selected,
        interactive,
        |entry| {
            let name = entry.directory.as_str();

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
    )
}

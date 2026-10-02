use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::config::load_workspace;
use crate::progress::{run_entries, RepoStatus};
use crate::store::Sources;

/// Push the topic branch of the root and of every checkout on it, each under
/// the same name on its own remote, as its upstream. Nothing else is pushed: a
/// checkout off the topic sits at a pin, and off a topic there is nothing of
/// gitscale's to push.
pub fn run(
    root: Option<&Path>,
    names: &[String],
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    let sources = Sources::new(&config_root, false)?;
    let resolution =
        crate::resolve::workspace(&config, &config_root, false, &sources, None, false)?;
    let selected = resolution.select(names)?;
    let pushed = run_entries(
        "Pushing local changes...",
        "push",
        &selected,
        interactive,
        |entry| {
            let name = entry.directory.as_str();
            let dest = config_root.join(&entry.directory);
            if entry.is_artefact() && !crate::git::is_checkout(&dest) {
                return RepoStatus::Skip(format!("{} (artefact)", name));
            }
            if !crate::git::is_checkout(&dest) || dest.is_symlink() {
                return RepoStatus::Skip(format!("{} (not a checkout of its own)", name));
            }
            let branch = resolution
                .slot(name)
                .and_then(|s| s.topic.as_ref())
                .map(|t| t.branch.clone())
                .filter(|b| crate::git::current_branch(&dest).as_deref() == Some(b.as_str()));
            let Some(branch) = branch else {
                return RepoStatus::Skip(format!("{} (not on the topic)", name));
            };
            match crate::git::push_topic(Some(entry), &dest, &branch) {
                Ok(true) => RepoStatus::Ok(format!("{} → {}", name, branch)),
                Ok(false) => RepoStatus::Skip(format!("{} (up to date)", name)),
                Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
            }
        },
        out,
        err,
    );
    let topic = crate::topic::root(&config, &config_root, false);
    let root = if names.is_empty() {
        match topic.topic() {
            None => Ok(()),
            Some(branch) => match crate::git::push_topic(None, &config_root, branch) {
                Ok(true) => {
                    writeln!(out, "  ok    . (workspace root) → {}", branch)?;
                    Ok(())
                }
                Ok(false) => {
                    writeln!(out, "  skip  . (workspace root, up to date)")?;
                    Ok(())
                }
                Err(e) => {
                    writeln!(out, "  FAIL  . (workspace root): {}", e)?;
                    Err(anyhow::anyhow!("the workspace root failed to push"))
                }
            },
        }
    } else {
        Ok(())
    };
    pushed.and(root)
}

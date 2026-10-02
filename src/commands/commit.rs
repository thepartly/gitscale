use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use crate::config::load_workspace;
use crate::git::commit_path;
use crate::progress::{run_parallel, RepoStatus};
use crate::store::Sources;

/// Commit, with one message, the root and every checkout on the workspace's
/// topic that has changes. A checkout off the topic is never committed — its
/// HEAD is detached at a pin — and one with changes says how to bring it in.
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

    let (config, config_root) = load_workspace(root)?;
    let sources = Sources::new(&config_root, false)?;
    let topic = crate::topic::root(&config, &config_root, false);
    let resolution =
        crate::resolve::workspace(&config, &config_root, false, &sources, None, false)?;
    let selected = resolution.select(names)?;
    let dir_names: Vec<String> = selected.iter().map(|e| e.directory.clone()).collect();
    let entries: HashMap<&str, &crate::config::RepoEntry> =
        selected.iter().map(|e| (e.directory.as_str(), e)).collect();

    let mut failed = run_parallel(
        "Committing local changes...",
        &dir_names,
        interactive,
        |name| {
            let entry = entries[name];
            let dest = config_root.join(&entry.directory);
            if !crate::git::is_checkout(&dest) {
                return RepoStatus::Skip(format!("{} (not cloned)", name));
            }
            // A dependency's link to a checkout committed under its own name.
            if dest.is_symlink() {
                return RepoStatus::Skip(format!("{} (symlink)", name));
            }
            let on_topic = resolution
                .slot(name)
                .and_then(|s| s.topic.as_ref())
                .is_some_and(|t| crate::git::current_branch(&dest).as_deref() == Some(&t.branch));
            if !on_topic {
                let planted = resolution.planted_in(name);
                if crate::git::uncommitted(&dest, &planted).is_none() {
                    return RepoStatus::Skip(format!("{} (clean)", name));
                }
                return RepoStatus::Skip(match topic.topic() {
                    Some(branch) => format!(
                        "{} (not on topic {}; run gitscale develop {})",
                        name, branch, name
                    ),
                    None => format!(
                        "{} (at its pin; switch the root to a topic branch, then gitscale \
                         develop {})",
                        name, name
                    ),
                });
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

    if names.is_empty() {
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

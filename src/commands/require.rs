//! `git scale require` / `unrequire`: add a dependency to the root's
//! `.gitscale.toml`, or take one out, editing the file in place — comments,
//! key order and tables gitscale does not know about are kept — and then
//! place the workspace, so the checkout comes or goes with the entry.

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::commands::sync::{place, Placement};
use crate::config::{check_entry, load_config, ArtefactUse, RepoEntry, CONFIG_FILENAME};
use crate::resolve::Network;

/// The root the entry goes in, and its directory relative to it. With no
/// config anywhere, the root is the top of the repository `start` is in:
/// `require` makes the config.
fn locate(start: Option<&Path>, dir: &str) -> Result<(PathBuf, String)> {
    let root = match crate::config::find_root(start) {
        Ok(root) => root,
        Err(_) => {
            let from = match start {
                Some(p) => p.to_path_buf(),
                None => std::env::current_dir()?,
            };
            let top = crate::git::query(&from, &["rev-parse", "--show-toplevel"])
                .ok_or_else(|| anyhow::anyhow!("{}", crate::config::NOT_IN_WORKSPACE))?;
            PathBuf::from(top)
        }
    };
    let here = crate::paths::Here::new(start, &root)?;
    let rel = here.relative(dir)?;
    if rel.as_os_str().is_empty() {
        bail!("the root is not a checkout");
    }
    Ok((here.root, rel.to_string_lossy().into_owned()))
}

#[allow(clippy::too_many_arguments)]
pub fn require(
    start: Option<&Path>,
    dir: &str,
    url: &str,
    revision: &str,
    artefact: Option<&str>,
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let artefact = artefact.map(ArtefactUse::from_str_checked).transpose()?;
    let (root, directory) = locate(start, dir)?;
    let path = root.join(CONFIG_FILENAME);
    // Held to the rules a loaded config is, so that `require` cannot write a
    // config every later command refuses to read; and a config that does not
    // read now is left as it is.
    check_entry(&directory, url, revision, &path)?;
    let config = if path.is_file() {
        load_config(&path)?
    } else {
        Default::default()
    };
    if config.repos.iter().any(|e| e.directory == directory) {
        bail!("{} is already declared in {}", directory, path.display());
    }
    let entry = RepoEntry {
        directory: directory.clone(),
        repo_url: url.to_string(),
        revision: revision.to_string(),
        artefact,
        ..Default::default()
    };
    crate::config::add_entry(&path, &entry)?;
    let config = load_config(&path).with_context(|| {
        format!(
            "{} no longer reads after adding {}",
            path.display(),
            directory
        )
    })?;
    let at = if revision.is_empty() {
        String::new()
    } else {
        format!(" @ {}", revision)
    };
    match artefact {
        Some(artefact) => writeln!(
            out,
            "Required {} → {}{} [artefact {}]",
            directory, url, at, artefact
        )?,
        None => writeln!(out, "Required {} → {}{}", directory, url, at)?,
    }
    place(
        &config,
        &root,
        &Placement {
            dirs: &[],
            network: Network::Online,
            force: false,
            leave: None,
            heading: "Pulling latest changes...",
        },
        verbose,
        no_cache,
        interactive,
        out,
        err,
    )
}

pub fn unrequire(
    start: Option<&Path>,
    dir: &str,
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (root, directory) = locate(start, dir)?;
    let path = root.join(CONFIG_FILENAME);
    if !path.is_file() {
        bail!("{}", crate::config::NOT_IN_WORKSPACE);
    }
    // A config that does not read is left untouched.
    load_config(&path)?;
    if !crate::config::remove_entry(&path, &directory)? {
        bail!("{} is not declared in {}", directory, path.display());
    }
    let config = load_config(&path)?;
    writeln!(out, "Unrequired {}", directory)?;
    // The checkout goes with the relink step, unless it holds work.
    place(
        &config,
        &root,
        &Placement {
            dirs: &[],
            network: Network::Online,
            force: false,
            leave: None,
            heading: "Pulling latest changes...",
        },
        verbose,
        no_cache,
        interactive,
        out,
        err,
    )
}

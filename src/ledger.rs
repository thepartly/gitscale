//! The checkouts gitscale manages in a workspace, kept so it can tell which
//! directories are its own once nothing asks for them any more.
//!
//! A checkout stops being needed when its entry is removed or renamed in the
//! root config, or when no repository asks for an implicit dependency any
//! more. The config alone cannot say what used to be there; this record can.
//! `clone` and `pull` write down every checkout of the workspace that is on
//! disk — so a workspace made before the record existed is covered from its
//! first pull — and `sync` removes the ones resolution no longer wants.
//!
//! It lives beside the artefact install records: in the workspace's git
//! directory, per worktree, or `.gitscale/` beside a config that is not the
//! top of a repository. Never inside a checkout.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::RepoEntry;

const FILE: &str = "checkouts.json";

/// What a recorded checkout is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Recorded {
    Git,
    Artefact,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Ledger {
    /// By directory, relative to the config.
    checkouts: BTreeMap<String, Recorded>,
}

fn path(config_root: &Path) -> PathBuf {
    let in_git = if crate::git::is_repo_root(config_root) {
        crate::git::git_path(config_root, &format!("gitscale/{}", FILE))
    } else {
        None
    };
    in_git.unwrap_or_else(|| config_root.join(".gitscale").join(FILE))
}

fn load(config_root: &Path) -> Ledger {
    fs::read(path(config_root))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save(config_root: &Path, ledger: &Ledger) -> Result<()> {
    let file = path(config_root);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let partial = file.with_extension("partial");
    fs::write(&partial, serde_json::to_vec_pretty(ledger)?)
        .with_context(|| format!("cannot write {}", partial.display()))?;
    fs::rename(&partial, &file).with_context(|| format!("cannot write {}", file.display()))
}

/// What is at `directory` now, if it is a checkout gitscale could have made:
/// a git checkout of its own, or an installed artefact. A symlink is somebody
/// else's checkout, and never recorded.
fn found(config_root: &Path, entry_dir: &str) -> Option<Recorded> {
    let dest = config_root.join(entry_dir);
    if dest.is_symlink() {
        return None;
    }
    if crate::git::is_checkout(&dest) {
        return Some(Recorded::Git);
    }
    crate::artefact::installed(config_root, entry_dir).map(|_| Recorded::Artefact)
}

/// Record every checkout of `entries` that is on disk now.
pub fn record(config_root: &Path, entries: &[RepoEntry]) -> Result<()> {
    let mut ledger = load(config_root);
    let before = ledger.checkouts.len();
    let mut changed = false;
    for entry in entries {
        if let Some(kind) = found(config_root, &entry.directory) {
            if ledger.checkouts.insert(entry.directory.clone(), kind) != Some(kind) {
                changed = true;
            }
        }
    }
    if changed || ledger.checkouts.len() != before {
        save(config_root, &ledger)?;
    }
    Ok(())
}

/// Recorded checkouts no entry of `entries` is any more, that are still on
/// disk as what was recorded — and that hold no checkout still wanted inside
/// them.
pub fn left_behind(config_root: &Path, entries: &[RepoEntry]) -> Vec<(String, Recorded)> {
    let ledger = load(config_root);
    ledger
        .checkouts
        .into_iter()
        .filter(|(dir, kind)| {
            !entries
                .iter()
                .any(|e| e.directory == *dir || Path::new(&e.directory).starts_with(Path::new(dir)))
                && found(config_root, dir) == Some(*kind)
        })
        .collect()
}

/// Every git checkout recorded, whether or not it is still wanted.
pub fn git_checkouts(config_root: &Path) -> Vec<String> {
    load(config_root)
        .checkouts
        .into_iter()
        .filter(|(_, kind)| *kind == Recorded::Git)
        .map(|(dir, _)| dir)
        .collect()
}

/// Forget `directory`: removed, or found to be something else now.
pub fn forget(config_root: &Path, directory: &str) -> Result<()> {
    let mut ledger = load(config_root);
    if ledger.checkouts.remove(directory).is_some() {
        save(config_root, &ledger)?;
    }
    Ok(())
}

/// Drop what is recorded but no longer on disk as recorded, so the record
/// does not grow with every checkout a workspace ever had.
pub fn prune(config_root: &Path) -> Result<()> {
    let mut ledger = load(config_root);
    let before = ledger.checkouts.len();
    ledger
        .checkouts
        .retain(|dir, kind| found(config_root, dir) == Some(*kind));
    if ledger.checkouts.len() != before {
        save(config_root, &ledger)?;
    }
    Ok(())
}

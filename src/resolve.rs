use anyhow::Result;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::artefact::Artefacts;
use crate::config::{GitScaleConfig, RepoEntry};
use crate::resolution::{Engine, Resolution};
use crate::store::Sources;
use crate::stores::{GitStores, WorkspaceCheckouts};

/// A symlink to create after cloning.
#[derive(Debug, Clone)]
pub struct SymlinkEntry {
    /// Path relative to workspace root where the symlink is created.
    pub link_path: PathBuf,
    /// Path relative to workspace root that the symlink points to.
    pub target_path: PathBuf,
}

/// The declared entry a dependency link sits under: the one whose directory is
/// the longest whole-component prefix of `link_path`, however many components
/// that directory spans.
pub fn owning_entry<'a>(link_path: &Path, entries: &'a [RepoEntry]) -> Option<&'a RepoEntry> {
    entries
        .iter()
        .filter(|e| link_path != Path::new(&e.directory) && link_path.starts_with(&e.directory))
        .max_by_key(|e| Path::new(&e.directory).components().count())
}

/// Resolve the workspace at `config_root`: every checkout it needs, at the
/// revision each one gets, and the links between them — on the root's topic,
/// when it is on one. See [`crate::resolution`].
///
/// `online` fetches what resolution reads first — once per repository per
/// command, into the store the checkout itself is a worktree of. Offline, it
/// works from what this machine already has, and what that cannot answer
/// comes back as an unresolved slot.
pub fn workspace(
    config: &GitScaleConfig,
    config_root: &Path,
    online: bool,
    sources: &Sources,
    artefacts: Option<&Artefacts>,
    verbose: bool,
) -> Result<Resolution> {
    let root_url = crate::git::origin_url(config_root);
    let stores = GitStores::new(config_root, sources, online, artefacts, verbose);
    let checkouts = WorkspaceCheckouts::new(config_root);
    let topic = crate::topic::root(config, config_root, online);
    Engine::new(config, root_url.as_deref(), &stores, &checkouts)
        .with_topic(topic.topic().map(str::to_string))
        .resolve()
}

/// Create symlinks on disk. Skips entries whose target doesn't exist yet.
pub fn create_symlinks(symlinks: &[SymlinkEntry], config_root: &Path) -> Result<()> {
    for entry in symlinks {
        let link_abs = config_root.join(&entry.link_path);
        let target_abs = config_root.join(&entry.target_path);

        if !target_abs.exists() {
            continue;
        }

        let expected_rel = relative_path(
            entry.link_path.parent().unwrap_or(Path::new("")),
            &entry.target_path,
        );

        // Already correct
        if link_abs.is_symlink() {
            let existing = fs::read_link(&link_abs)?;
            if existing == expected_rel {
                continue;
            }
            fs::remove_file(&link_abs)?;
        } else if link_abs.exists() {
            // Real file/directory — don't clobber
            continue;
        }

        if let Some(parent) = link_abs.parent() {
            fs::create_dir_all(parent)?;
        }

        std::os::unix::fs::symlink(&expected_rel, &link_abs)?;
        // Write directly to stderr so it appears in real-time alongside
        // interactive progress bars, not buffered until the end.
        eprintln!(
            "  link  {} → {}",
            entry.link_path.display(),
            entry.target_path.display(),
        );
    }
    Ok(())
}

/// An orphaned symlink: a gitscale-managed symlink that is no longer declared
/// in any config.
#[derive(Debug, Clone)]
pub struct OrphanLink {
    /// Path (relative to `config_root`) of the orphaned symlink.
    pub link_path: PathBuf,
    /// True when the symlink target no longer exists on disk.
    pub broken: bool,
}

/// Find gitscale-managed symlinks that are no longer declared in config.
///
/// A symlink qualifies as an orphan when it lives inside a recursive repo's
/// checkout, is relative and points at what gitscale links to — a direct child
/// of the config root, a checkout of the workspace, or anything under the
/// hoist directory — and is not in the current `expected` symlink set. This
/// heuristic avoids removing a user's own internal symlinks.
pub fn find_orphan_links(
    entries: &[RepoEntry],
    config_root: &Path,
    expected: &[SymlinkEntry],
    hoist_dir: &str,
) -> Vec<OrphanLink> {
    let expected_set: std::collections::HashSet<&Path> =
        expected.iter().map(|e| e.link_path.as_path()).collect();
    let targets = Targets {
        root: config_root.to_path_buf(),
        checkouts: entries
            .iter()
            .map(|e| config_root.join(&e.directory))
            .collect(),
        hoist: config_root.join(hoist_dir),
    };

    let mut orphans = Vec::new();
    for entry in entries {
        if !entry.recursive {
            continue;
        }
        let repo_dir = config_root.join(&entry.directory);
        if !repo_dir.is_dir() || repo_dir.is_symlink() {
            continue;
        }
        scan_orphans(&repo_dir, &targets, &expected_set, &mut orphans);
    }
    orphans
}

/// What a gitscale link may point at.
struct Targets {
    root: PathBuf,
    checkouts: std::collections::HashSet<PathBuf>,
    hoist: PathBuf,
}

impl Targets {
    fn covers(&self, resolved: &Path) -> bool {
        resolved.parent() == Some(self.root.as_path())
            || self.checkouts.contains(resolved)
            || (resolved.starts_with(&self.hoist) && resolved != self.hoist)
    }
}

fn scan_orphans(
    dir: &Path,
    targets: &Targets,
    expected_set: &std::collections::HashSet<&Path>,
    orphans: &mut Vec<OrphanLink>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name() == ".git" {
            continue;
        }
        let path = entry.path();
        let is_symlink = path
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if is_symlink {
            // Never descend into symlinks; just classify them.
            if let Some(orphan) = classify_orphan(&path, targets, expected_set) {
                orphans.push(orphan);
            }
        } else if path.is_dir() {
            scan_orphans(&path, targets, expected_set, orphans);
        }
    }
}

fn classify_orphan(
    link_abs: &Path,
    targets: &Targets,
    expected_set: &std::collections::HashSet<&Path>,
) -> Option<OrphanLink> {
    let link_rel = link_abs.strip_prefix(&targets.root).ok()?;
    if expected_set.contains(link_rel) {
        return None;
    }
    let target = fs::read_link(link_abs).ok()?;
    // gitscale only ever creates relative symlinks.
    if target.is_absolute() {
        return None;
    }
    let resolved = lexical_join(link_abs.parent()?, &target);
    if !targets.covers(&resolved) {
        return None;
    }
    Some(OrphanLink {
        link_path: link_rel.to_path_buf(),
        broken: !link_abs.exists(),
    })
}

/// Whether the symlink at `link_abs`, an entry path of the workspace at
/// `config_root`, is a dedup link an enclosing workspace planted: relative,
/// as gitscale makes them, and pointing out of this workspace. That checkout,
/// and the revision it sits at, belong to the outer workspace's root config —
/// which is how running gitscale inside a child repository finds its deps.
pub fn is_outer_link(link_abs: &Path, config_root: &Path) -> bool {
    let Ok(target) = fs::read_link(link_abs) else {
        return false;
    };
    let Some(parent) = link_abs.parent() else {
        return false;
    };
    !target.is_absolute() && !lexical_join(parent, &target).starts_with(config_root)
}

/// Lexically join `base` with `rel`, resolving `.` and `..` components without
/// touching the filesystem (so it works for broken symlinks).
fn lexical_join(base: &Path, rel: &Path) -> PathBuf {
    let mut result: Vec<Component> = base.components().collect();
    for comp in rel.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(result.last(), Some(Component::Normal(_))) {
                    result.pop();
                } else {
                    result.push(comp);
                }
            }
            other => result.push(other),
        }
    }
    result.iter().collect()
}

/// Compute relative path from directory `from` to path `to`.
/// Both paths are relative to the same root.
fn relative_path(from: &Path, to: &Path) -> PathBuf {
    let from_parts: Vec<Component> = from.components().collect();
    let to_parts: Vec<Component> = to.components().collect();

    let common = from_parts
        .iter()
        .zip(to_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut result = PathBuf::new();
    for _ in common..from_parts.len() {
        result.push("..");
    }
    for part in &to_parts[common..] {
        result.push(part);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relative_path_sibling() {
        let from = Path::new("repoA/libs");
        let to = Path::new("repoB");
        assert_eq!(relative_path(from, to), PathBuf::from("../../repoB"));
    }

    #[test]
    fn test_relative_path_shared_prefix() {
        let from = Path::new("libs/repoA/deps");
        let to = Path::new("libs/repoB");
        assert_eq!(relative_path(from, to), PathBuf::from("../../repoB"));
    }

    #[test]
    fn test_relative_path_root_level() {
        let from = Path::new("");
        let to = Path::new("repoB");
        assert_eq!(relative_path(from, to), PathBuf::from("repoB"));
    }

    #[test]
    fn test_lexical_join_parent() {
        // `../sibling` from a repo dir resolves to a config-root sibling.
        assert_eq!(
            lexical_join(Path::new("/root/repoA"), Path::new("../repoB")),
            PathBuf::from("/root/repoB")
        );
        // Nested dep path `../../repo` resolves the same way.
        assert_eq!(
            lexical_join(Path::new("/root/repoA/libs"), Path::new("../../repoB")),
            PathBuf::from("/root/repoB")
        );
        // CurDir components are ignored.
        assert_eq!(
            lexical_join(Path::new("/root/repoA"), Path::new("./x")),
            PathBuf::from("/root/repoA/x")
        );
    }
}

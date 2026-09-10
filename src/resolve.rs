use anyhow::{bail, Result};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::config::{load_config, RepoEntry, CONFIG_FILENAME};
use crate::git::checkout_revision;

/// A symlink to create after cloning.
#[derive(Debug, Clone)]
pub struct SymlinkEntry {
    /// Path relative to workspace root where the symlink is created.
    pub link_path: PathBuf,
    /// Path relative to workspace root that the symlink points to.
    pub target_path: PathBuf,
}

/// Scan cloned repos for nested `.gitscale.toml` files, validate that all
/// transitive dependencies are declared at root level, resolve empty revisions,
/// and return the list of symlinks to create.
///
/// Returns `(symlinks, resolved_revisions)` where `resolved_revisions` is a
/// list of `(directory, revision)` pairs for root repos whose revision was
/// empty and got adopted from a child config.
pub fn resolve_recursive(
    root_repos: &[RepoEntry],
    config_root: &Path,
) -> Result<(Vec<SymlinkEntry>, Vec<(String, String)>)> {
    let mut symlinks = Vec::new();
    // Track revision adoption: normalized_url -> (revision, source_child_directory)
    let mut adopted: HashMap<String, (String, String)> = HashMap::new();

    // Build normalized URL -> root entry lookup
    let url_to_root: HashMap<String, &RepoEntry> = root_repos
        .iter()
        .map(|e| (crate::urls::normalize(&e.repo_url), e))
        .collect();

    for root_entry in root_repos {
        if !root_entry.recursive {
            continue;
        }

        let child_config_path = config_root
            .join(&root_entry.directory)
            .join(CONFIG_FILENAME);

        if !child_config_path.is_file() {
            continue;
        }

        let child_config = load_config(&child_config_path)?;

        for child_dep in &child_config.repos {
            let norm_url = crate::urls::normalize(&child_dep.repo_url);

            let Some(matched_root) = url_to_root.get(&norm_url) else {
                bail!(
                    "repo '{}' requires '{}' (url: {}) but it is not declared in the root {}",
                    root_entry.directory,
                    child_dep.directory,
                    child_dep.repo_url,
                    CONFIG_FILENAME,
                );
            };

            // Revision resolution: if root has no opinion, adopt child's
            if matched_root.revision.is_empty() && !child_dep.revision.is_empty() {
                if let Some((existing_rev, existing_source)) = adopted.get(&norm_url) {
                    if *existing_rev != child_dep.revision {
                        bail!(
                            "conflicting revisions for '{}': '{}' wants '{}', '{}' wants '{}'. \
                             Pin a revision in the root {} to resolve.",
                            child_dep.repo_url,
                            existing_source,
                            existing_rev,
                            root_entry.directory,
                            child_dep.revision,
                            CONFIG_FILENAME,
                        );
                    }
                } else {
                    adopted.insert(
                        norm_url,
                        (child_dep.revision.clone(), root_entry.directory.clone()),
                    );
                }
            }

            // Record symlink from child's dep path to root's checkout path
            let link_path = PathBuf::from(&root_entry.directory).join(&child_dep.directory);
            let target_path = PathBuf::from(&matched_root.directory);

            symlinks.push(SymlinkEntry {
                link_path,
                target_path,
            });
        }
    }

    // Convert adopted map to (directory, revision) pairs
    let resolved_revisions: Vec<(String, String)> = adopted
        .into_iter()
        .filter_map(|(norm_url, (rev, _))| {
            url_to_root
                .get(&norm_url)
                .map(|e| (e.directory.clone(), rev))
        })
        .collect();

    Ok((symlinks, resolved_revisions))
}

/// Create symlinks on disk. Skips entries whose target doesn't exist yet.
pub fn create_symlinks(
    symlinks: &[SymlinkEntry],
    config_root: &Path,
    out: &mut dyn Write,
) -> Result<()> {
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
/// checkout, follows gitscale's relative-link-to-config-root-sibling pattern
/// (e.g. `../repo` or `../../repo`), and is not in the current `expected`
/// symlink set. This heuristic avoids removing a user's own internal symlinks.
pub fn find_orphan_links(
    root_repos: &[RepoEntry],
    config_root: &Path,
    expected: &[SymlinkEntry],
) -> Vec<OrphanLink> {
    let expected_set: std::collections::HashSet<&Path> =
        expected.iter().map(|e| e.link_path.as_path()).collect();

    let mut orphans = Vec::new();
    for entry in root_repos {
        if !entry.recursive {
            continue;
        }
        let repo_dir = config_root.join(&entry.directory);
        if !repo_dir.is_dir() || repo_dir.is_symlink() {
            continue;
        }
        scan_orphans(&repo_dir, config_root, &expected_set, &mut orphans);
    }
    orphans
}

fn scan_orphans(
    dir: &Path,
    config_root: &Path,
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
            if let Some(orphan) = classify_orphan(&path, config_root, expected_set) {
                orphans.push(orphan);
            }
        } else if path.is_dir() {
            scan_orphans(&path, config_root, expected_set, orphans);
        }
    }
}

fn classify_orphan(
    link_abs: &Path,
    config_root: &Path,
    expected_set: &std::collections::HashSet<&Path>,
) -> Option<OrphanLink> {
    let link_rel = link_abs.strip_prefix(config_root).ok()?;
    if expected_set.contains(link_rel) {
        return None;
    }
    let target = fs::read_link(link_abs).ok()?;
    // gitscale only ever creates relative symlinks.
    if target.is_absolute() {
        return None;
    }
    let resolved = lexical_join(link_abs.parent()?, &target);
    // gitscale links always point to a direct child of the config root.
    if resolved.parent() != Some(config_root) {
        return None;
    }
    Some(OrphanLink {
        link_path: link_rel.to_path_buf(),
        broken: !link_abs.exists(),
    })
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

/// Checkout resolved revisions for repos that had an empty revision in root
/// config but got one adopted from a child.
pub fn apply_resolved_revisions(
    revisions: &[(String, String)],
    root_repos: &[RepoEntry],
    config_root: &Path,
) -> Result<()> {
    for (directory, revision) in revisions {
        let original = root_repos
            .iter()
            .find(|e| e.directory == *directory)
            .unwrap();
        let resolved_entry = RepoEntry {
            directory: directory.clone(),
            repo_url: original.repo_url.clone(),
            revision: revision.clone(),
            mode: original.mode,
            recursive: original.recursive,
        };
        let dest = config_root.join(directory);
        if dest.exists() {
            checkout_revision(&resolved_entry, config_root)?;
        }
    }
    Ok(())
}

/// High-level: resolve transitive deps and create symlinks.
/// When `apply_revisions` is true, also checkout adopted revisions.
pub fn resolve_and_link(
    root_repos: &[RepoEntry],
    config_root: &Path,
    apply_revisions: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let (symlinks, resolved_revisions) = resolve_recursive(root_repos, config_root)?;

    if apply_revisions && !resolved_revisions.is_empty() {
        apply_resolved_revisions(&resolved_revisions, root_repos, config_root)?;
    }

    if !symlinks.is_empty() {
        create_symlinks(&symlinks, config_root, out)?;
    }

    Ok(())
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

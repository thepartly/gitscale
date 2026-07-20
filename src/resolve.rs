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
        .map(|e| (normalize_url(&e.repo_url), e))
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
            let norm_url = normalize_url(&child_dep.repo_url);

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

/// Canonicalize a git remote URL so that different transport forms of the same
/// repository (e.g. `git@github.com:org/repo.git` and
/// `https://github.com/org/repo`) compare as equal.
///
/// When the host and owner/repo can be extracted, the canonical form is
/// `host/owner/repo` (lowercased). Otherwise we fall back to stripping a
/// trailing `.git` and lowercasing.
fn normalize_url(url: &str) -> String {
    match (
        crate::urls::extract_hostname(url),
        crate::urls::extract_owner_repo(url),
    ) {
        (Ok(host), Ok((owner, repo))) => format!("{}/{}/{}", host, owner, repo).to_lowercase(),
        _ => url.strip_suffix(".git").unwrap_or(url).to_lowercase(),
    }
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
    fn test_normalize_url() {
        // SSH and HTTPS forms of the same repo canonicalize identically.
        assert_eq!(
            normalize_url("git@github.com:org/repo.git"),
            "github.com/org/repo"
        );
        assert_eq!(
            normalize_url("https://github.com/ORG/Repo"),
            "github.com/org/repo"
        );
        assert_eq!(
            normalize_url("git@github.com:org/repo.git"),
            normalize_url("https://github.com/org/repo.git")
        );
        // Unparseable URLs fall back to strip-.git + lowercase.
        assert_eq!(normalize_url("file:///Tmp/Repo.git"), "file:///tmp/repo");
    }
}

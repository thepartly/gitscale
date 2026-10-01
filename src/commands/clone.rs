use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::Path;

use crate::cache::Cache;
use crate::commands::cache::Sources;
use crate::config::{filter_entries, load_workspace, CacheSettings, RepoEntry, CONFIG_FILENAME};
use crate::git::{clone_repo, pull_repo};
use crate::progress::{run_entries, RepoStatus};
use crate::resolve::is_outer_link;
use crate::storage::clone_artefact;

pub fn run(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    if let Some((url, directory)) = bootstrap_target(names)? {
        return bootstrap(
            root,
            url,
            directory,
            verbose,
            no_cache,
            interactive,
            out,
            err,
        );
    }

    let (config, config_root) = load_workspace(root)?;
    let selected = filter_entries(&config.repos, names)?;

    if selected.is_empty() {
        writeln!(out, "Nothing to clone.")?;
        return Ok(());
    }

    let storage_url = &config.storage_url;
    let sources = Sources::adopting(&config, &config_root, no_cache, verbose, out)?;

    run_entries(
        "Cloning missing repos...",
        "clone",
        &selected,
        interactive,
        |entry| {
            let name = entry.directory.as_str();
            let dest = config_root.join(&entry.directory);

            // Replace symlinks with actual clones — except, unless the entry
            // was named, an enclosing workspace's dedup link: run inside a
            // child repository, that is the outer root's checkout, and
            // unlinking it is a choice to make one dependency at a time.
            if dest.is_symlink() {
                if names.is_empty() && is_outer_link(&dest, &config_root) {
                    return RepoStatus::Skip(format!("{} (symlink)", name));
                }
                if let Err(e) = std::fs::remove_file(&dest) {
                    return RepoStatus::Fail(format!("{}: failed to remove symlink: {}", name, e));
                }
            }

            if entry.is_artefact() {
                if dest.exists() {
                    return RepoStatus::Skip(format!("{} (already exists)", name));
                }
                if storage_url.is_empty() {
                    return RepoStatus::Fail(format!("{}: no [storage] configured", name));
                }
                return match clone_artefact(storage_url, &entry.repo_url, &entry.revision, &dest) {
                    Ok(true) => RepoStatus::Ok(format!("{} (artefact)", name)),
                    Ok(false) => RepoStatus::Skip(format!("{} (no artefact data)", name)),
                    Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
                };
            }

            if crate::git::is_checkout(&dest) {
                return RepoStatus::Skip(format!("{} (already exists)", name));
            }

            // Cache first: the entry talks to the remote, then the workspace
            // is built from whatever that left on local disk.
            let from = sources.for_entry(entry);
            match clone_repo(entry, &config_root, verbose, &from) {
                Ok(()) => RepoStatus::Ok(name.to_string()),
                Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
            }
        },
        out,
        err,
    )?;

    let adopted = crate::resolve::resolve_and_link(&config.repos, &config_root)?;
    move_to_adopted(&config.repos, &config_root, adopted, &sources, verbose)
}

/// Move each root entry with no revision of its own to the one a child pins,
/// the way `pull` moves any checkout: readonly files stay readonly, a shallow
/// clone fetches the ref it was never cloned at, and CI's cache pins that
/// revision rather than the default branch.
pub(crate) fn move_to_adopted(
    repos: &[RepoEntry],
    config_root: &Path,
    adopted: impl IntoIterator<Item = (String, String)>,
    sources: &Sources,
    verbose: bool,
) -> Result<()> {
    for (directory, revision) in adopted {
        let Some(original) = repos.iter().find(|e| e.directory == directory) else {
            continue;
        };
        let dest = config_root.join(&directory);
        // An artefact has no git checkout to move, and a symlink is someone
        // else's checkout.
        if original.is_artefact() || dest.is_symlink() || !crate::git::is_checkout(&dest) {
            continue;
        }
        let entry = RepoEntry {
            revision,
            ..original.clone()
        };
        let from = sources.for_entry(&entry);
        pull_repo(&entry, config_root, verbose, &from).with_context(|| {
            format!(
                "{}: moving to adopted revision '{}'",
                directory, entry.revision
            )
        })?;
    }
    Ok(())
}

/// The URL and directory a bootstrapping `gitscale clone <url> [directory]`
/// names, or `None` when the arguments are the usual list of declared repos.
fn bootstrap_target(names: &[String]) -> Result<Option<(&str, Option<&str>)>> {
    let Some(first) = names.first() else {
        return Ok(None);
    };
    if !crate::urls::looks_like_remote(first) {
        return Ok(None);
    }
    if names.len() > 2 {
        bail!(
            "clone from a URL takes at most a directory to create, but got: {}",
            names[1..].join(" ")
        );
    }
    Ok(Some((first.as_str(), names.get(1).map(String::as_str))))
}

/// Bootstrap a workspace from cold.
///
/// The root repository is treated like any other: its objects go through the
/// cache, so the *second* workspace of it costs nothing over the wire — and
/// the clone is linked to the cache from birth, which is what makes adopting
/// unnecessary here.
#[allow(clippy::too_many_arguments)]
fn bootstrap(
    root: Option<&Path>,
    url: &str,
    directory: Option<&str>,
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let parent = match root {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir()?,
    };
    let name = directory
        .map(str::to_string)
        .unwrap_or_else(|| default_directory(url));
    let dest = parent.join(name);
    if dest.exists() {
        bail!("Directory already exists: {}", dest.display());
    }

    // There is no config to read yet, so the cache is whatever the environment
    // and `--no-cache` say. A `[cache]` table in the repository being cloned
    // governs everything from the next step on.
    let settings = CacheSettings {
        enabled: !no_cache,
        ..CacheSettings::default()
    };
    let cache = Cache::open(&settings);
    let reference = match &cache {
        Some(cache) => match cache.mirror(url) {
            Ok(path) => Some(path),
            Err(e) => {
                writeln!(err, "  cache {}: {} (using the remote)", url, e)?;
                None
            }
        },
        None => None,
    };

    writeln!(out, "Cloning {} into {}...", url, dest.display())?;
    crate::git::clone_url(url, &dest, reference.as_deref(), verbose)?;

    if !dest.join(CONFIG_FILENAME).is_file() {
        writeln!(
            out,
            "No {} in the clone; nothing further to do.",
            CONFIG_FILENAME
        )?;
        return Ok(());
    }
    run(Some(&dest), &[], verbose, no_cache, interactive, out, err)
}

/// The directory a URL clones into by default, the way `git clone` picks one.
fn default_directory(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    let name = trimmed
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(trimmed)
        .trim_end_matches(".git");
    if name.is_empty() {
        "workspace".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_argument_bootstraps_but_a_directory_name_does_not() {
        let names = vec!["https://github.com/org/repo.git".to_string()];
        assert_eq!(
            bootstrap_target(&names).unwrap(),
            Some(("https://github.com/org/repo.git", None))
        );
        let names = vec!["libs/core".to_string()];
        assert_eq!(bootstrap_target(&names).unwrap(), None);
        assert_eq!(bootstrap_target(&[]).unwrap(), None);
    }

    #[test]
    fn a_bootstrap_takes_one_directory_at_most() {
        let names = vec![
            "git@github.com:org/repo.git".to_string(),
            "ws".to_string(),
            "extra".to_string(),
        ];
        assert!(bootstrap_target(&names).is_err());
    }

    #[test]
    fn the_default_directory_is_the_repository_name() {
        assert_eq!(default_directory("https://github.com/org/repo.git"), "repo");
        assert_eq!(default_directory("git@github.com:org/repo.git"), "repo");
        assert_eq!(default_directory("/srv/mirrors/repo.git/"), "repo");
    }
}

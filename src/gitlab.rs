//! GitLab Runner's own checkout, as far as it concerns an installed git hook.
//!
//! The runner prepares a job's build directory in this order: fetch, `git
//! checkout -f <sha>`, then `git clean $GIT_CLEAN_FLAGS`. `post-checkout` fires
//! at the second step, so everything the hook-triggered pull puts in the
//! working tree is still in front of the clean — and the default flags,
//! `-ffdx`, delete every declared checkout: they are untracked, usually
//! ignored, and `-ff` removes nested repositories too. The job then starts with
//! no sub-repositories, and fails somewhere far from the cause.
//!
//! Nothing gitscale does can run after that clean, so the fix belongs in the
//! pipeline (`GIT_CLEAN_FLAGS: -ffdx -e /imports/`) and gitscale's part is to
//! refuse to let a missing one go unnoticed.

use anyhow::{bail, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::config::{load_workspace, RepoEntry};
use crate::git::{clean_report, run_git};

/// What the runner uses when a pipeline does not set `GIT_CLEAN_FLAGS`.
const DEFAULT_CLEAN_FLAGS: &str = "-ffdx";

/// Fail if the runner's post-checkout clean would delete a declared checkout.
///
/// A no-op outside a GitLab job. Run after the hook's pull, when the checkouts
/// exist: the check asks git itself — `git clean -n` with the runner's exact
/// flags — so every combination of flags, `.gitignore` rules and exclude
/// patterns means what it will mean to the real clean a moment later.
pub fn check_runner_clean(root: &Path) -> Result<()> {
    check_runner_clean_with(root, &|name| std::env::var(name).ok())
}

fn check_runner_clean_with(root: &Path, var: &dyn Fn(&str) -> Option<String>) -> Result<()> {
    if var("GITLAB_CI").as_deref() != Some("true") {
        return Ok(());
    }
    let flags = var("GIT_CLEAN_FLAGS").unwrap_or_else(|| DEFAULT_CLEAN_FLAGS.to_string());
    // `none` is the runner's own spelling for "do not clean".
    if flags.trim() == "none" {
        return Ok(());
    }

    let (config, config_root) = load_workspace(Some(root))?;

    // The runner cleans from the top of the checkout, which is not necessarily
    // where the config sits.
    let top = run_git(&["rev-parse", "--show-toplevel"], Some(&config_root), true)?;
    let top = PathBuf::from(String::from_utf8_lossy(&top.stdout).trim());
    let prefix = match (config_root.canonicalize(), top.canonicalize()) {
        (Ok(config_root), Ok(top)) => config_root
            .strip_prefix(&top)
            .map(Path::to_path_buf)
            .unwrap_or_default(),
        _ => PathBuf::new(),
    };

    let mut args = vec!["clean", "-n"];
    args.extend(flags.split_whitespace());
    let listed = run_git(&args, Some(&top), true)?;
    let removed: Vec<PathBuf> = clean_report(&listed)
        .iter()
        .map(|path| PathBuf::from(path.trim_end_matches('/')))
        .collect();

    let doomed = doomed_checkouts(&config.repos, &prefix, &removed);
    if doomed.is_empty() {
        return Ok(());
    }
    bail!(
        "GIT_CLEAN_FLAGS=\"{}\" would delete {} — GitLab Runner cleans after its checkout, \
         so the job would start without {}.\n\
         Exclude the checkouts from that clean, in .gitlab-ci.yml or the runner's environment:\n\n    \
         GIT_CLEAN_FLAGS: {}",
        flags.trim(),
        doomed.join(", "),
        if doomed.len() == 1 { "it" } else { "them" },
        suggested_flags(&flags, &declared_checkouts(&config.repos, &prefix)),
    )
}

/// Every declared checkout, relative to the checkout top.
fn declared_checkouts(repos: &[RepoEntry], prefix: &Path) -> Vec<String> {
    repos
        .iter()
        .map(|entry| prefix.join(&entry.directory).display().to_string())
        .collect()
}

/// The declared checkouts, relative to the checkout top, that a removed path
/// takes with it — the checkout itself, or a directory holding it.
fn doomed_checkouts(repos: &[RepoEntry], prefix: &Path, removed: &[PathBuf]) -> Vec<String> {
    repos
        .iter()
        .map(|entry| prefix.join(&entry.directory))
        .filter(|dir| removed.iter().any(|gone| dir.starts_with(gone)))
        .map(|dir| dir.display().to_string())
        .collect()
}

/// The line the error asks for: the flags as they are, plus `-e /<top>/` for
/// every top-level directory holding a declared checkout that the flags do not
/// already exclude by that name.
///
/// Built from all the checkouts rather than only those this clean would take:
/// the directory is what gets excluded, so the suggestion covers every
/// checkout in it — and one the next commit adds there — in one pattern.
fn suggested_flags(flags: &str, checkouts: &[String]) -> String {
    let excluded = exclude_patterns(flags);
    let tops: BTreeSet<String> = checkouts
        .iter()
        .filter_map(|dir| Path::new(dir).components().next())
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .filter(|top| !excluded.contains(top))
        .collect();
    let mut suggested = flags.trim().to_string();
    for top in tops {
        suggested.push_str(&format!(" -e /{}/", top));
    }
    suggested
}

/// The `-e` patterns in `flags`, in every spelling `git clean` accepts, with
/// the anchoring slash and the directory slash dropped — `/imports/`,
/// `imports/` and `imports` all name the same directory for this purpose.
fn exclude_patterns(flags: &str) -> BTreeSet<String> {
    let mut patterns = BTreeSet::new();
    let mut words = flags.split_whitespace();
    while let Some(word) = words.next() {
        let pattern = match word {
            "-e" | "--exclude" => words.next(),
            _ => word
                .strip_prefix("--exclude=")
                .or_else(|| word.strip_prefix("-e").filter(|p| !p.is_empty())),
        };
        if let Some(pattern) = pattern {
            patterns.insert(pattern.trim_matches('/').to_string());
        }
    }
    patterns
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(directory: &str) -> RepoEntry {
        RepoEntry {
            directory: directory.to_string(),
            repo_url: String::new(),
            revision: String::new(),
            mode: crate::config::RepoMode::Readwrite,
            recursive: true,
        }
    }

    #[test]
    fn a_removed_parent_takes_the_checkout_with_it() {
        let repos = [entry("imports/a"), entry("imports/b"), entry("vendor/c")];
        let removed = [PathBuf::from("imports"), PathBuf::from("target")];
        assert_eq!(
            doomed_checkouts(&repos, Path::new(""), &removed),
            ["imports/a", "imports/b"]
        );
    }

    #[test]
    fn a_sibling_with_a_shared_prefix_is_not_a_parent() {
        let repos = [entry("imports-extra/a")];
        let removed = [PathBuf::from("imports")];
        assert!(doomed_checkouts(&repos, Path::new(""), &removed).is_empty());
    }

    fn dirs(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn suggests_one_exclude_per_top_level_directory() {
        assert_eq!(
            suggested_flags("-ffdx", &dirs(&["imports/a", "imports/b", "vendor/c"])),
            "-ffdx -e /imports/ -e /vendor/"
        );
    }

    #[test]
    fn a_directory_already_excluded_is_not_suggested_again() {
        let checkouts = dirs(&["imports/a", "vendor/c"]);
        for flags in [
            "-ffdx -e /imports/",
            "-ffdx -e imports",
            "-ffdx -e/imports/",
            "-ffdx --exclude=/imports/",
            "-ffdx --exclude imports/",
        ] {
            assert_eq!(
                suggested_flags(flags, &checkouts),
                format!("{} -e /vendor/", flags),
                "{}",
                flags
            );
        }
    }

    #[test]
    fn an_exclude_of_one_checkout_still_gets_its_directory() {
        // `-e /imports/a` keeps one checkout; the directory is what covers them all.
        assert_eq!(
            suggested_flags("-ffdx -e /imports/a", &dirs(&["imports/a", "imports/b"])),
            "-ffdx -e /imports/a -e /imports/"
        );
    }

    #[test]
    fn outside_gitlab_there_is_nothing_to_check() {
        // No config is needed: the check returns before looking for one.
        let nowhere = Path::new("/nonexistent/gitscale-test");
        assert!(check_runner_clean_with(nowhere, &|_| None).is_ok());
    }

    #[test]
    fn a_runner_told_not_to_clean_needs_no_exclude() {
        let nowhere = Path::new("/nonexistent/gitscale-test");
        let var = |name: &str| match name {
            "GITLAB_CI" => Some("true".to_string()),
            "GIT_CLEAN_FLAGS" => Some("none".to_string()),
            _ => None,
        };
        assert!(check_runner_clean_with(nowhere, &var).is_ok());
    }
}

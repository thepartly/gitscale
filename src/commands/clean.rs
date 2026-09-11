use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::commands::clone::filter_entries;
use crate::config::{find_config, load_config, RepoEntry, CONFIG_FILENAME};
use crate::git::{clean_repo, is_repo_root};
use crate::progress::{run_parallel, RepoStatus};
use crate::resolve::resolve_recursive;

/// The name `status` gives the workspace repo itself, reused here so the same
/// string addresses it on the command line.
const SELF_NAME: &str = ".";

/// Always kept, in every repo cleaned. A `.gitscale.toml` is usually tracked
/// and so out of reach anyway, but one that has been written and not yet
/// committed is untracked — and deleting the file that defines the workspace
/// is not a thing a clean should be able to do to you.
const ALWAYS_KEEP: &str = "/.gitscale.toml";

/// One repo to clean, with the exclude set that applies to it.
struct Target {
    /// The declared directory, or `.` for the workspace repo.
    name: String,
    dir: PathBuf,
    excludes: Vec<String>,
    /// Set when there is nothing to do, carrying the reason to report.
    skip: Option<String>,
}

pub fn run(
    root: Option<&Path>,
    names: &[String],
    cli_excludes: &[String],
    force: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;

    for pattern in cli_excludes {
        if pattern.is_empty() {
            bail!("--exclude needs a pattern");
        }
        if pattern.starts_with('-') {
            bail!(
                "--exclude pattern \"{}\" starts with '-', which git would read as a \
                 command-line option",
                pattern
            );
        }
    }

    let targets = plan(&config, &config_root, names, cli_excludes)?;
    if targets.is_empty() {
        writeln!(out, "Nothing to clean.")?;
        return Ok(());
    }

    if force {
        execute(&targets, interactive, out, err)
    } else {
        report(&targets, out)
    }
}

/// Work out what to clean and what each repo keeps.
fn plan(
    config: &crate::config::GitScaleConfig,
    config_root: &Path,
    names: &[String],
    cli_excludes: &[String],
) -> Result<Vec<Target>> {
    // `.` addresses the workspace repo; every other name must be a declared
    // repo. An empty selection means all of them, the root included.
    let want_self = names.is_empty() || names.iter().any(|n| n == SELF_NAME);
    let repo_names: Vec<String> = names.iter().filter(|n| *n != SELF_NAME).cloned().collect();
    let selected = if names.is_empty() {
        config.repos.clone()
    } else if repo_names.is_empty() {
        Vec::new()
    } else {
        filter_entries(&config.repos, &repo_names)?
    };

    // A config `resolve` cannot make sense of is a config whose symlink set is
    // unknown, and the links are the thing standing between a clean and a
    // broken workspace. Refuse rather than guess.
    let managed_links = managed_link_excludes(&config.repos, config_root)?;

    let mut targets = Vec::new();

    if want_self {
        if is_repo_root(config_root) {
            // Every declared checkout is an untracked directory as far as the
            // workspace repo is concerned, so without these exclusions a clean
            // at the root would delete the workspace it was run in.
            let mut excludes = vec![ALWAYS_KEEP.to_string()];
            excludes.extend(config.clean.exclude.iter().cloned());
            excludes.extend(cli_excludes.iter().cloned());
            excludes.extend(nested_checkouts(&config.repos, Path::new("")));
            targets.push(Target {
                name: SELF_NAME.to_string(),
                dir: config_root.to_path_buf(),
                excludes,
                skip: None,
            });
        } else if !names.is_empty() {
            // Only worth reporting when it was asked for by name: a workspace
            // that is not itself a repo is an ordinary setup, not a problem.
            targets.push(skipped(SELF_NAME, config_root, "not a git repository"));
        }
    }

    for entry in &selected {
        let dir = config_root.join(&entry.directory);
        let name = entry.directory.as_str();

        if !entry.recursive {
            // Its config is not gitscale's to read, so its keep-list is
            // unknown — and cleaning a repo without knowing what it wants
            // kept is worse than leaving it alone.
            targets.push(skipped(name, &dir, "recursive = false"));
            continue;
        }
        if entry.is_artefact() {
            targets.push(skipped(name, &dir, "artefact"));
            continue;
        }
        if !dir.exists() {
            targets.push(skipped(name, &dir, "not cloned"));
            continue;
        }
        if dir.is_symlink() {
            // The checkout lives outside the workspace and belongs to whoever
            // put it there.
            targets.push(skipped(name, &dir, "symlink"));
            continue;
        }
        if !is_repo_root(&dir) {
            targets.push(skipped(name, &dir, "not a git repository"));
            continue;
        }

        let mut excludes = vec![ALWAYS_KEEP.to_string()];
        excludes.extend(cli_excludes.iter().cloned());
        excludes.extend(own_excludes(entry, &dir)?);
        // A repo declared inside this one is a checkout in its own right, and
        // cleaning is not how it gets removed.
        excludes.extend(nested_checkouts(&config.repos, Path::new(&entry.directory)));
        if let Some(links) = managed_links.get(&entry.directory) {
            excludes.extend(links.iter().cloned());
        }
        targets.push(Target {
            name: name.to_string(),
            dir,
            excludes,
            skip: None,
        });
    }

    Ok(targets)
}

/// Anchored patterns for the checkouts that land inside `holder`, which is the
/// workspace root when empty.
///
/// Every one of them is an untracked directory to the repo holding it, so
/// without these a clean deletes the checkouts it was run to tidy. git happens
/// to refuse to delete a directory that is itself a git repository unless
/// given a second `-f`, but that is git declining to do something reckless
/// rather than gitscale knowing what it owns — an artefact checkout has no
/// `.git` to recognise it by, and would go.
fn nested_checkouts(repos: &[RepoEntry], holder: &Path) -> Vec<String> {
    repos
        .iter()
        .filter_map(|repo| {
            let inner = Path::new(&repo.directory).strip_prefix(holder).ok()?;
            // `strip_prefix` also succeeds against the repo itself, leaving
            // nothing: that is the working tree being cleaned, not something
            // sitting inside it.
            (!inner.as_os_str().is_empty()).then_some(())?;
            Some(format!("/{}", inner.display()))
        })
        .collect()
}

fn skipped(name: &str, dir: &Path, reason: &str) -> Target {
    Target {
        name: name.to_string(),
        dir: dir.to_path_buf(),
        excludes: Vec::new(),
        skip: Some(reason.to_string()),
    }
}

/// A repo's own `[clean] exclude`, read from the `.gitscale.toml` in its
/// checkout. Only reached for repos `plan` has established are gitscale's to
/// descend into.
///
/// The root's `[clean]` is deliberately not merged in — a sub-repository knows
/// its own build outputs, and a root config enumerating them on its behalf
/// goes stale the moment the sub-repository changes.
///
/// A config that fails to parse is an error rather than an empty list:
/// treating an unreadable file as "keep nothing" would delete exactly the
/// files it was written to protect.
fn own_excludes(entry: &RepoEntry, dir: &Path) -> Result<Vec<String>> {
    let child_config = dir.join(CONFIG_FILENAME);
    if !child_config.is_file() {
        return Ok(Vec::new());
    }
    let config = load_config(&child_config)
        .with_context(|| format!("{}: cannot read its clean rules", entry.directory))?;
    Ok(config.clean.exclude)
}

/// Anchored exclude patterns for the symlinks `resolve` plants inside each
/// recursive repo, keyed by the repo directory holding them. From git's point
/// of view those links are untracked files, so a clean would take them with
/// it and leave the child repo's declared dependency paths dangling.
fn managed_link_excludes(
    repos: &[RepoEntry],
    config_root: &Path,
) -> Result<HashMap<String, Vec<String>>> {
    let (symlinks, _) = resolve_recursive(repos, config_root)?;

    // Longest directory first, so a link inside `libs/core` is attributed to
    // that repo rather than to a `libs` that also happens to be declared.
    let mut by_depth: Vec<&RepoEntry> = repos.iter().collect();
    by_depth.sort_by_key(|e| std::cmp::Reverse(Path::new(&e.directory).components().count()));

    let mut by_repo: HashMap<String, Vec<String>> = HashMap::new();
    for link in &symlinks {
        for repo in &by_depth {
            if let Ok(inner) = link.link_path.strip_prefix(&repo.directory) {
                by_repo
                    .entry(repo.directory.clone())
                    .or_default()
                    .push(format!("/{}", inner.display()));
                break;
            }
        }
    }
    Ok(by_repo)
}

/// List what would go, without touching anything.
fn report(targets: &[Target], out: &mut dyn Write) -> Result<()> {
    writeln!(out, "Clean (dry run — nothing removed; pass -f to delete)")?;

    let mut total = 0usize;
    let mut repos = 0usize;
    for target in targets {
        if let Some(reason) = &target.skip {
            writeln!(out, "  {} — skip ({})", target.name, reason)?;
            continue;
        }
        let paths = clean_repo(&target.dir, &target.excludes, false)
            .with_context(|| format!("{}: cannot list untracked files", target.name))?;
        if paths.is_empty() {
            writeln!(out, "  {} — nothing to remove", target.name)?;
            continue;
        }
        repos += 1;
        total += paths.len();
        writeln!(out, "  {}", target.name)?;
        for path in &paths {
            writeln!(out, "    {}", path)?;
        }
    }

    if total == 0 {
        writeln!(out, "\nNothing to remove.")?;
    } else {
        writeln!(
            out,
            "\n{} in {}.",
            count(total, "path"),
            count(repos, "repo")
        )?;
    }
    Ok(())
}

/// Remove the files.
///
/// Readonly repos need no special handling: `apply_readonly` clears the write
/// bit on files but leaves directories alone, and unlinking a file needs write
/// permission on its directory rather than on the file itself.
fn execute(
    targets: &[Target],
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let by_name: HashMap<&str, &Target> = targets.iter().map(|t| (t.name.as_str(), t)).collect();
    let names: Vec<String> = targets.iter().map(|t| t.name.clone()).collect();

    let failed = run_parallel(
        "Removing untracked files...",
        &names,
        interactive,
        |name| {
            let target = &by_name[name];
            if let Some(reason) = &target.skip {
                return RepoStatus::Skip(format!("{} ({})", name, reason));
            }
            match clean_repo(&target.dir, &target.excludes, true) {
                Ok(paths) if paths.is_empty() => {
                    RepoStatus::Skip(format!("{} (nothing to remove)", name))
                }
                Ok(paths) => RepoStatus::Ok(format!("{} ({})", name, count(paths.len(), "path"))),
                Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
            }
        },
        out,
        err,
    )?;

    if failed > 0 {
        bail!("{} repo(s) failed to clean", failed);
    }
    Ok(())
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("{} {}", n, noun)
    } else {
        format!("{} {}s", n, noun)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RepoMode;

    fn repo(directory: &str) -> RepoEntry {
        RepoEntry {
            directory: directory.to_string(),
            repo_url: "https://example.com/r.git".to_string(),
            revision: "main".to_string(),
            mode: RepoMode::Readwrite,
            recursive: true,
        }
    }

    #[test]
    fn the_workspace_root_holds_every_declared_checkout() {
        let repos = [repo("core"), repo("libs/utils")];
        assert_eq!(
            nested_checkouts(&repos, Path::new("")),
            vec!["/core", "/libs/utils"]
        );
    }

    #[test]
    fn a_repo_holds_the_checkouts_declared_inside_it_but_not_itself() {
        let repos = [repo("core"), repo("core/plugins"), repo("coreutils")];
        // `coreutils` shares a textual prefix with `core` without being inside
        // it, so the match has to be by path component rather than by string.
        assert_eq!(
            nested_checkouts(&repos, Path::new("core")),
            vec!["/plugins"]
        );
    }

    #[test]
    fn a_leaf_repo_holds_nothing() {
        let repos = [repo("core"), repo("libs/utils")];
        assert!(nested_checkouts(&repos, Path::new("libs/utils")).is_empty());
    }
}

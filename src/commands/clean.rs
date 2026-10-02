use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::{filter_entries, load_config, load_workspace, RepoEntry, CONFIG_FILENAME};
use crate::git::{clean_repo, is_repo_root};
use crate::progress::{run_parallel, RepoStatus};
use crate::store::Sources;

/// How the workspace repo itself is named on the command line.
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
    /// An entry's directory with no repository in it, removed whole. See
    /// [`stray`].
    stray: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    root: Option<&Path>,
    names: &[String],
    cli_excludes: &[String],
    gc: bool,
    keep_recent: Option<&str>,
    force: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    let sources = Sources::new(&config_root, false)?;
    if let Some(stores) = &sources.stores {
        stores.tidy(&config_root);
        if gc {
            return compact(stores, &config, keep_recent, out);
        }
    } else if gc {
        bail!("--gc compacts the root's own stores, and CI keeps none; use gitscale cache compact");
    }

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

    let targets = plan(&config, &config_root, &sources, names, cli_excludes)?;
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

/// `gitscale clean --gc`: `git gc` in every store of the root, and the images
/// nothing has used within `keep_recent` dropped now rather than on the next
/// day's pull.
fn compact(
    stores: &crate::store::Stores,
    config: &crate::config::GitScaleConfig,
    keep_recent: Option<&str>,
    out: &mut dyn Write,
) -> Result<()> {
    let keep = crate::store::keep_recent(keep_recent.or(config.clean.keep_recent.as_deref()))?;
    let before = crate::store::dir_size(stores.root());
    let collected = stores.gc()?;
    let pruned = stores.images().prune(keep)?;
    let after = crate::store::dir_size(stores.root());
    writeln!(
        out,
        "Compacted {}: {} collected, {} dropped, {} freed.",
        stores.root().display(),
        crate::cache::plural(collected, "store", "stores"),
        crate::cache::plural(pruned.images + pruned.entries, "image", "images"),
        crate::cache::human_size(before.saturating_sub(after))
    )?;
    Ok(())
}

/// Work out what to clean and what each repo keeps.
fn plan(
    config: &crate::config::GitScaleConfig,
    config_root: &Path,
    sources: &Sources,
    names: &[String],
    cli_excludes: &[String],
) -> Result<Vec<Target>> {
    // `.` addresses the workspace repo; every other name must be a declared
    // repo. An empty selection means all of them, the root included.
    // A config `resolve` cannot make sense of is a config whose symlink set —
    // and whose implicit checkouts — are unknown, and those are the things
    // standing between a clean and a broken workspace. Refuse rather than
    // guess. Offline: a clean never fetches.
    let resolution = crate::resolve::workspace(config, config_root, false, sources, None, false)?;
    // Every checkout, the implicit ones included: each is an untracked
    // directory to whatever repo holds it.
    let all = resolution.entries();

    let want_self = names.is_empty() || names.iter().any(|n| n == SELF_NAME);
    let repo_names: Vec<String> = names.iter().filter(|n| *n != SELF_NAME).cloned().collect();
    let selected = if names.is_empty() {
        all.clone()
    } else if repo_names.is_empty() {
        Vec::new()
    } else {
        filter_entries(&all, &repo_names)?
    };

    let managed_links = managed_link_excludes(&resolution, &all);

    let mut targets = Vec::new();

    if want_self {
        // Every declared checkout is an untracked directory as far as the
        // workspace repo is concerned, so without these exclusions a clean at
        // the root would delete the workspace it was run in.
        let mut excludes = vec![ALWAYS_KEEP.to_string()];
        excludes.extend(config.clean.exclude.iter().cloned());
        excludes.extend(cli_excludes.iter().cloned());
        excludes.extend(nested_checkouts(&all, Path::new("")));
        targets.push(Target {
            name: SELF_NAME.to_string(),
            dir: config_root.to_path_buf(),
            excludes,
            skip: None,
            stray: false,
        });
    }

    for entry in &selected {
        let dir = config_root.join(&entry.directory);
        let name = entry.directory.as_str();

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
        if !crate::git::is_checkout(&dir) {
            targets.push(stray(&all, entry, &dir));
            continue;
        }
        if !is_repo_root(&dir) {
            // It has a `.git`, so it is a checkout of some kind, if a broken
            // one — and a broken checkout can still hold the only copy of
            // somebody's work.
            targets.push(skipped(name, &dir, "not a git repository"));
            continue;
        }

        let mut excludes = vec![ALWAYS_KEEP.to_string()];
        excludes.extend(cli_excludes.iter().cloned());
        // `recursive = false` means its config is not gitscale's to read, keep-
        // list included, so the repo is cleaned by the command line's patterns
        // alone. Its own nested dependencies are safe without that config:
        // gitscale never planted any, and a clone someone made there is a
        // nested repository, which `clean` reports rather than deletes.
        if entry.recursive {
            excludes.extend(own_excludes(entry, &dir)?);
        }
        // A repo declared inside this one is a checkout in its own right, and
        // cleaning is not how it gets removed.
        excludes.extend(nested_checkouts(&all, Path::new(&entry.directory)));
        if let Some(links) = managed_links.get(&entry.directory) {
            excludes.extend(links.iter().cloned());
        }
        // An overlay's files are ignored build output, and exactly what the
        // overlay is for.
        if entry.is_overlay() {
            excludes.extend(
                crate::artefact::overlay_files(config_root, &entry.directory)
                    .into_iter()
                    .map(|file| format!("/{}", file)),
            );
        }
        targets.push(Target {
            name: name.to_string(),
            dir,
            excludes,
            skip: None,
            stray: false,
        });
    }

    Ok(targets)
}

/// An entry's directory that exists but holds no repository: left by a failed
/// clone, an interrupted delete, an outside cleaner, or a CI cache restored
/// into a path whose checkout was not.
///
/// It is removed whole. Nothing in it belongs to a checkout, `pull` refuses to
/// clone over it, and git cannot clean it — there is no repository to run
/// `git clean` in, and the workspace's own clean excludes the path. Left
/// alone, a clean would leave behind the one thing standing between it and a
/// working `pull`. The exception is a directory holding another declared
/// checkout that is on disk: removing it would take that checkout with it.
fn stray(all: &[RepoEntry], entry: &RepoEntry, dir: &Path) -> Target {
    let holds_checkout = nested_checkouts(all, Path::new(&entry.directory))
        .iter()
        .any(|inner| {
            dir.join(inner.trim_start_matches('/'))
                .symlink_metadata()
                .is_ok()
        });
    if holds_checkout {
        return skipped(
            &entry.directory,
            dir,
            "holds no repository, but another declared checkout is inside it",
        );
    }
    Target {
        name: entry.directory.clone(),
        dir: dir.to_path_buf(),
        excludes: Vec::new(),
        skip: None,
        stray: true,
    }
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
        stray: false,
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
    resolution: &crate::resolution::Resolution,
    repos: &[RepoEntry],
) -> HashMap<String, Vec<String>> {
    let symlinks = &resolution.links;

    // Longest directory first, so a link inside `libs/core` is attributed to
    // that repo rather than to a `libs` that also happens to be declared.
    let mut by_depth: Vec<&RepoEntry> = repos.iter().collect();
    by_depth.sort_by_key(|e| std::cmp::Reverse(Path::new(&e.directory).components().count()));

    let mut by_repo: HashMap<String, Vec<String>> = HashMap::new();
    for link in symlinks {
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
    by_repo
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
        if target.stray {
            repos += 1;
            total += 1;
            writeln!(out, "  {}", target.name)?;
            writeln!(out, "    ./ (the whole directory: it holds no repository)")?;
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
            crate::cache::plural(total, "path", "paths"),
            crate::cache::plural(repos, "repo", "repos")
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
            if target.stray {
                return match std::fs::remove_dir_all(&target.dir) {
                    Ok(()) => RepoStatus::Ok(format!("{} (removed: it held no repository)", name)),
                    Err(e) => RepoStatus::Fail(format!("{}: cannot remove: {}", name, e)),
                };
            }
            match clean_repo(&target.dir, &target.excludes, true) {
                Ok(paths) if paths.is_empty() => {
                    RepoStatus::Skip(format!("{} (nothing to remove)", name))
                }
                Ok(paths) => RepoStatus::Ok(format!(
                    "{} ({})",
                    name,
                    crate::cache::plural(paths.len(), "path", "paths")
                )),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(directory: &str) -> RepoEntry {
        RepoEntry {
            directory: directory.to_string(),
            repo_url: "https://example.com/r.git".to_string(),
            revision: "main".to_string(),
            recursive: true,
            ..Default::default()
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

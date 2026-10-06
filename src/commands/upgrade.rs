//! `git upgrade`: raise pins, on a topic, in two forms that differ only in
//! where the new revision comes from.
//!
//! * `upgrade` — promotion. For each slot of the topic whose change is in the
//!   newest release of its pin's kind and major, that tag is written into the
//!   topic's configs that ask for less, its topic branch is deleted on its
//!   remote, and the slot leaves the topic.
//! * `upgrade <dir>...` — the newest release of each named dependency,
//!   written into the topic's configs that ask for less. A requester not on
//!   the topic is named, with the command that joins it.
//!
//! Only the root and the checkouts on the topic are edited: joining is
//! `git topic join`'s, and a topic begins only with `git topic start`. This is
//! the one command that looks for newer tags; resolution never does. Files are
//! edited with `toml_edit`, so comments and key order survive, and committed
//! only with `--commit`.

use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::artefact::Artefacts;
use crate::checkout::Placer;
use crate::config::{load_workspace, set_revisions, GitScaleConfig, RepoEntry, CONFIG_FILENAME};
use crate::paths::Here;
use crate::promote::{self, below, Containment, State};
use crate::resolution::{Resolution, Slot};
use crate::store::{Sources, Stores};

pub struct Options<'a> {
    pub dirs: &'a [String],
    pub major: bool,
    pub commit: bool,
    pub dry_run: bool,
}

/// One revision to write into one config.
#[derive(Debug, Clone)]
struct Edit {
    /// The repository whose `.gitscale.toml` it is: the workspace root, or a
    /// checkout.
    repo: PathBuf,
    /// The config as the output names it.
    label: String,
    /// The entry's key in that config.
    key: String,
    from: String,
    to: String,
    /// The dependency it raises, for the commit message.
    dependency: String,
}

pub fn run(
    root: Option<&Path>,
    opts: &Options,
    verbose: bool,
    no_cache: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    let here = Here::new(root, &config_root)?;
    let sources = Sources::new(&config_root, no_cache)?;
    if sources.stores.is_none() {
        bail!(
            "upgrade edits configs in the checkouts of a developer machine; CI keeps none to \
             edit"
        );
    }
    if opts.dirs.is_empty() && opts.major {
        bail!(
            "--major raises a named dependency: git upgrade --major <dir>. Promotion stays in \
             the major each pin is in"
        );
    }
    let Some(branch) = crate::topic::root(&config, &config_root, true)
        .topic()
        .map(str::to_string)
    else {
        bail!("not on a topic: git topic start NAME");
    };
    let artefacts = Artefacts::new(&config, &config_root, sources.images());
    let resolution = crate::resolve::workspace(
        &config,
        &config_root,
        true,
        &sources,
        Some(&artefacts),
        verbose,
    )?;
    if opts.dirs.is_empty() {
        promote_topic(
            &config,
            &config_root,
            &sources,
            &artefacts,
            &resolution,
            &branch,
            opts,
            out,
        )
    } else {
        let stores = sources.stores.as_ref().expect("checked above");
        raise(
            &config,
            &config_root,
            &here,
            stores,
            &artefacts,
            &resolution,
            opts,
            out,
        )
    }
}

// ---------------------------------------------------------------------------
// Promotion
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn promote_topic(
    config: &GitScaleConfig,
    config_root: &Path,
    sources: &Sources,
    artefacts: &Artefacts,
    resolution: &Resolution,
    branch: &str,
    opts: &Options,
    out: &mut dyn Write,
) -> Result<()> {
    let topic = resolution.topic_slots();
    if topic.is_empty() {
        writeln!(out, "Nothing on topic {} to promote.", branch)?;
        return Ok(());
    }

    let stores = sources.stores.as_ref().expect("checked by run");

    // Resolution has just fetched every store, tags included: those are what
    // is asked.
    let mut states: BTreeMap<String, State> = BTreeMap::new();
    for slot in &topic {
        let planted = resolution.planted_in(&slot.directory);
        let url = slot.url.clone();
        let has_image = move |tag: &str| artefacts.has_image(&url, tag);
        let store = stores.repo_path(&crate::ci::remote_url(&slot.url));
        let state = promote::assess(config_root, &store, slot, &planted, Some(&has_image));
        states.insert(slot.directory.clone(), state);
    }
    let promoted: BTreeSet<String> = states
        .iter()
        .filter(|(_, s)| s.is_promoted())
        .map(|(d, _)| d.clone())
        .collect();
    let requests = promote::topic_requests(config, config_root, resolution);

    let mut edits = Vec::new();
    let mut leaving = Vec::new();
    for slot in &topic {
        let state = &states[&slot.directory];
        writeln!(out, "{}  {}", slot.directory, state.describe())?;
        let State::Promoted(tag) = state else {
            continue;
        };
        // Every config of the topic that asks for this one, for less.
        let mut requesters: Vec<(PathBuf, String, Option<GitScaleConfig>)> = vec![(
            config_root.to_path_buf(),
            CONFIG_FILENAME.to_string(),
            Some(config.clone()),
        )];
        for other in &topic {
            if other.directory == slot.directory || !on_its_branch(config_root, other) {
                continue;
            }
            let dir = config_root.join(&other.directory);
            requesters.push((
                dir.clone(),
                format!("{}/{}", other.directory, CONFIG_FILENAME),
                promote::working_config(&dir),
            ));
        }
        for (repo, label, requester) in requesters {
            let Some(requester) = requester else {
                continue;
            };
            for entry in requester.repos.iter().filter(|e| same_repo(e, slot)) {
                match below(&entry.revision, tag) {
                    Some(true) if entry.is_override => writeln!(
                        out,
                        "  {}   {} held by override at {}; not changed",
                        label, entry.directory, entry.revision
                    )?,
                    Some(true) => edits.push(Edit {
                        repo: repo.clone(),
                        label: label.clone(),
                        key: entry.directory.clone(),
                        from: entry.revision.clone(),
                        to: tag.clone(),
                        dependency: slot.directory.clone(),
                    }),
                    _ => {}
                }
            }
        }
        leaving.push((*slot, tag.clone()));
    }

    // Before anything changes: a branch that can't be deleted without losing
    // work stops the promotion here.
    let branches = remote_branches(stores, &leaving)?;
    print_edits(&edits, out)?;
    let next = promote::next_to_merge(&requests, &promoted);
    if opts.dry_run {
        for remote in &branches {
            writeln!(
                out,
                "  {}: would delete origin/{}",
                remote.dir, remote.branch
            )?;
        }
        if !next.is_empty() {
            writeln!(out, "next to merge: {}", next.join(", "))?;
        }
        writeln!(out, "(dry run: nothing changed)")?;
        return Ok(());
    }
    // Deleted before the pin bump is written, so the pipeline of the bump
    // never builds from a branch that still matches.
    for remote in &branches {
        remote.delete(stores)?;
        writeln!(out, "  {}: deleted origin/{}", remote.dir, remote.branch)?;
    }
    let placer = Placer {
        config_root,
        sources,
        artefacts,
        verbose: false,
        fetch: true,
    };
    for (slot, tag) in &leaving {
        let left = leave_topic(config_root, resolution, &placer, slot, tag)?;
        writeln!(out, "  {}  left the topic: {}", slot.directory, left)?;
    }
    let committed = apply(&edits, opts.commit, out)?;
    if !next.is_empty() {
        writeln!(out, "next to merge: {}", next.join(", "))?;
    }
    to_push(config_root, &committed, out)
}

/// A promoted slot's topic branch on its remote, at the tip its release was
/// checked to hold.
struct RemoteBranch {
    dir: String,
    url: String,
    branch: String,
    tip: String,
}

impl RemoteBranch {
    /// Delete it on its remote — only while it is still at the tip checked,
    /// so a push made since fails the deletion instead of being lost.
    fn delete(&self, stores: &Stores) -> Result<()> {
        let lease = format!("--force-with-lease=refs/heads/{}:{}", self.branch, self.tip);
        crate::git::run_git(
            &[
                "push",
                "--quiet",
                &lease,
                &self.url,
                &format!(":refs/heads/{}", self.branch),
            ],
            Some(&stores.repo_path(&self.url)),
            true,
        )
        .with_context(|| {
            format!(
                "{}: cannot delete origin/{}; no config was changed. Run git upgrade again once \
                 it can be",
                self.dir, self.branch
            )
        })?;
        Ok(())
    }
}

/// The promoted slots' topic branches still on their remotes. Each must hold
/// nothing its release does not, or the promotion fails before anything
/// changes: deleting it would lose that work, and keeping it would leave
/// placement and CI building from it.
fn remote_branches(stores: &Stores, leaving: &[(&Slot, String)]) -> Result<Vec<RemoteBranch>> {
    let mut found = Vec::new();
    for (slot, tag) in leaving {
        let Some(topic) = &slot.topic else {
            continue;
        };
        let url = crate::ci::remote_url(&slot.url);
        let name = format!("refs/heads/{}", topic.branch);
        let Some((tip, _)) =
            crate::git::ls_remote_revision(&url, &name, true).with_context(|| {
                format!(
                    "{}: cannot ask its remote for {}",
                    slot.directory, topic.branch
                )
            })?
        else {
            continue;
        };
        let store = stores.repo_path(&url);
        if crate::git::resolve_ref(&store, &format!("{}^{{commit}}", tip)).is_none() {
            crate::git::run_git(&["fetch", "--quiet", &url, &name], Some(&store), true)
                .with_context(|| format!("{}: cannot fetch {}", slot.directory, topic.branch))?;
        }
        if promote::containment(&store, tag, &tip)? != Containment::Contained {
            bail!(
                "{}: origin/{} has changes {} does not hold; merge or drop them, then run again",
                slot.directory,
                topic.branch,
                tag
            );
        }
        found.push(RemoteBranch {
            dir: slot.directory.clone(),
            url,
            branch: topic.branch.clone(),
            tip,
        });
    }
    Ok(found)
}

/// The last line after `--commit`: the repositories that now have commits
/// to push — a promotion edits other topic checkouts' configs too.
fn to_push(config_root: &Path, committed: &[PathBuf], out: &mut dyn Write) -> Result<()> {
    if committed.is_empty() {
        return Ok(());
    }
    let mut names: Vec<String> = committed
        .iter()
        .map(|repo| name_of(config_root, repo))
        .collect();
    names.sort_by_key(|n| (n == ".", n.clone()));
    names.dedup();
    writeln!(out, "to push: {} — git scale push", names.join(", "))?;
    Ok(())
}

/// The repository at `repo` as the root names it: its directory, `.` for the
/// root itself.
fn name_of(config_root: &Path, repo: &Path) -> String {
    let rel = repo
        .strip_prefix(config_root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if rel.is_empty() {
        ".".to_string()
    } else {
        rel
    }
}

/// Take a promoted slot off the topic: its branch deleted, detached at the
/// tag that holds its change — or, for an artefact, that tag's image in its
/// place. One only following a remote branch has nothing here to leave.
fn leave_topic(
    config_root: &Path,
    resolution: &Resolution,
    placer: &Placer,
    slot: &Slot,
    tag: &str,
) -> Result<String> {
    let Some(topic) = slot.topic.as_ref().filter(|t| t.developed) else {
        return Ok(format!("pinned at {}", tag));
    };
    let dest = config_root.join(&slot.directory);
    let store = placer
        .sources
        .stores
        .as_ref()
        .expect("promotion runs with stores")
        .repo_path(&crate::ci::remote_url(&slot.url));
    let commit = crate::git::resolve_ref(&store, &format!("refs/tags/{}^{{commit}}", tag))
        .ok_or_else(|| anyhow::anyhow!("{}: tag {} is not fetched", slot.directory, tag))?;
    if crate::git::is_checkout(&dest) {
        crate::checkout::stop_branch(
            &dest,
            &topic.branch,
            &commit,
            &resolution.planted_in(&slot.directory),
            true,
        )
        .with_context(|| format!("{} stays on {}", slot.directory, topic.branch))?;
    }
    let placed = placer.place(&slot.off_topic(tag, &commit))?;
    Ok(format!("{} at {}", placed, tag))
}

/// Whether the checkout of `slot` is on its topic branch, so its working
/// tree's config is the topic's.
fn on_its_branch(config_root: &Path, slot: &Slot) -> bool {
    let dest = config_root.join(&slot.directory);
    slot.topic.as_ref().is_some_and(|t| {
        crate::git::is_checkout(&dest)
            && crate::git::current_branch(&dest).as_deref() == Some(t.branch.as_str())
    })
}

// ---------------------------------------------------------------------------
// Raising named dependencies
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn raise(
    config: &GitScaleConfig,
    config_root: &Path,
    here: &Here,
    stores: &Stores,
    artefacts: &Artefacts,
    resolution: &Resolution,
    opts: &Options,
    out: &mut dyn Write,
) -> Result<()> {
    let mut edits = Vec::new();
    for dir in opts.dirs {
        let slot = here.slot(resolution, dir)?;
        if let Some(entry) = config
            .repos
            .iter()
            .find(|e| e.directory == slot.directory && e.is_override)
        {
            writeln!(
                out,
                "{} is held at {} by override in {}; nothing changed",
                slot.directory, entry.revision, CONFIG_FILENAME
            )?;
            continue;
        }
        let target = promote::target(stores, artefacts, slot, opts.major)?;
        let (pin, tag) = match target.release(&slot.directory) {
            Ok((pin, tag)) => (pin.to_string(), tag.to_string()),
            Err(why) => {
                writeln!(out, "{}", why)?;
                continue;
            }
        };
        if !slot
            .requests
            .iter()
            .any(|r| below(&r.revision, &tag) == Some(true))
        {
            writeln!(
                out,
                "{} is already at its newest release, {}",
                slot.directory, pin
            )?;
            continue;
        }
        writeln!(out, "{}   {} → {}", slot.directory, pin, tag)?;
        for request in &slot.requests {
            let label = if request.from == "root" {
                CONFIG_FILENAME.to_string()
            } else {
                format!("{}/{}", request.from, CONFIG_FILENAME)
            };
            match below(&request.revision, &tag) {
                Some(true) => {}
                Some(false) => continue,
                None => {
                    writeln!(
                        out,
                        "  {}   {} asks for {}; not changed",
                        label,
                        request.directory,
                        if request.revision.is_empty() {
                            "no revision"
                        } else {
                            request.revision.as_str()
                        }
                    )?;
                    continue;
                }
            }
            if request.is_override {
                writeln!(
                    out,
                    "  {}   {} held by override at {}; not changed",
                    label, request.directory, request.revision
                )?;
                continue;
            }
            let repo = if request.from == "root" {
                config_root.to_path_buf()
            } else {
                let Some(requester) = resolution.slot(&request.from) else {
                    continue;
                };
                if config
                    .repos
                    .iter()
                    .any(|e| e.directory == requester.directory && e.is_override)
                {
                    writeln!(
                        out,
                        "  {}   held by override in {}; its pin is not changed",
                        requester.directory, CONFIG_FILENAME
                    )?;
                    continue;
                }
                if !on_its_branch(config_root, requester) {
                    writeln!(
                        out,
                        "  {}   {} asks for {}: git topic join {}",
                        label,
                        request.directory,
                        request.revision,
                        here.show(&config_root.join(&requester.directory))
                    )?;
                    continue;
                }
                config_root.join(&requester.directory)
            };
            edits.push(Edit {
                repo,
                label,
                key: request.directory.clone(),
                from: request.revision.clone(),
                to: tag.clone(),
                dependency: slot.directory.clone(),
            });
        }
    }

    print_edits(&edits, out)?;
    if opts.dry_run {
        writeln!(out, "(dry run: nothing changed)")?;
        return Ok(());
    }
    let committed = apply(&edits, opts.commit, out)?;
    let mut next: Vec<String> = Vec::new();
    for edit in &edits {
        let dir = name_of(config_root, &edit.repo);
        if !next.contains(&dir) {
            next.push(dir);
        }
    }
    if next.len() > 1 {
        next.retain(|d| d != ".");
    }
    if !next.is_empty() {
        writeln!(out, "next to merge: {}", next.join(", "))?;
    }
    to_push(config_root, &committed, out)
}

// ---------------------------------------------------------------------------
// Shared
// ---------------------------------------------------------------------------

/// Whether `entry` asks for the repository `slot` checks out.
fn same_repo(entry: &RepoEntry, slot: &Slot) -> bool {
    crate::urls::normalize(&entry.repo_url) == crate::urls::normalize(&slot.url)
}

fn print_edits(edits: &[Edit], out: &mut dyn Write) -> Result<()> {
    let lw = edits
        .iter()
        .map(|e| e.label.chars().count())
        .max()
        .unwrap_or(0);
    let kw = edits
        .iter()
        .map(|e| e.key.chars().count())
        .max()
        .unwrap_or(0);
    for edit in edits {
        writeln!(
            out,
            "  {:<lw$}   {:<kw$}  {} → {}",
            edit.label,
            edit.key,
            edit.from,
            edit.to,
            lw = lw,
            kw = kw,
        )?;
    }
    Ok(())
}

/// Write `edits`, one config at a time, and commit each with `commit`.
/// Returns the repositories committed in.
fn apply(edits: &[Edit], commit: bool, out: &mut dyn Write) -> Result<Vec<PathBuf>> {
    let mut by_repo: BTreeMap<&Path, Vec<(String, String)>> = BTreeMap::new();
    for edit in edits {
        by_repo
            .entry(edit.repo.as_path())
            .or_default()
            .push((edit.key.clone(), edit.to.clone()));
    }
    for (repo, changes) in &by_repo {
        set_revisions(&repo.join(CONFIG_FILENAME), changes)?;
    }
    if commit {
        return commit_configs(edits, out);
    }
    Ok(Vec::new())
}

/// Commit each edited `.gitscale.toml` — that file alone, whatever else the
/// repository has changed — as `pin <dependency> <tag>`. Returns the
/// repositories committed in.
fn commit_configs(edits: &[Edit], out: &mut dyn Write) -> Result<Vec<PathBuf>> {
    let mut by_repo: BTreeMap<&Path, (&str, Vec<String>)> = BTreeMap::new();
    for edit in edits {
        let item = format!("{} {}", edit.dependency, edit.to);
        let entry = by_repo
            .entry(edit.repo.as_path())
            .or_insert((edit.label.as_str(), Vec::new()));
        if !entry.1.contains(&item) {
            entry.1.push(item);
        }
    }
    let mut committed = Vec::new();
    for (repo, (label, items)) in by_repo {
        committed.push(repo.to_path_buf());
        let message = format!("pin {}", items.join(", "));
        crate::git::run_git(&["add", "--", CONFIG_FILENAME], Some(repo), true)?;
        crate::git::run_git(
            &["commit", "--quiet", "-m", &message, "--", CONFIG_FILENAME],
            Some(repo),
            true,
        )
        .with_context(|| format!("cannot commit {}", label))?;
        writeln!(out, "  commit  {}: {}", label, message)?;
    }
    Ok(committed)
}

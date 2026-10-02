//! `gitscale upgrade`: raise pins, in three forms that differ only in where
//! the new revision comes from.
//!
//! * `upgrade` — promotion. For each slot of the topic whose change is in the
//!   newest calver tag of its pin's stream, that tag is written into the
//!   topic's configs that ask for less, and the slot leaves the topic.
//! * `upgrade <dir>...` — the newest release of each named dependency,
//!   written into every config that asks for it; the repositories those
//!   configs belong to are developed on the topic, which is created — a new
//!   branch of the root — when there is none.
//! * `upgrade --resolved` — the revision resolution already selected,
//!   written into the root's own entries (what `gitscale resolve --write`
//!   did). No tag is looked up, and no topic is needed.
//!
//! This is the one command that looks for newer tags; resolution never does.
//! Files are edited with `toml_edit`, so comments and key order survive, and
//! committed only with `--commit`.

use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::artefact::Artefacts;
use crate::checkout::Placer;
use crate::config::{load_workspace, set_revisions, GitScaleConfig, RepoEntry, CONFIG_FILENAME};
use crate::promote::{self, State};
use crate::resolution::{Resolution, Slot};
use crate::store::Sources;
use crate::version;

pub struct Options<'a> {
    pub dirs: &'a [String],
    pub resolved: bool,
    pub major: bool,
    pub commit: bool,
    pub dry_run: bool,
    /// The topic branch to create when there is none (`-c`).
    pub create: Option<&'a str>,
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
    let sources = Sources::new(&config_root, no_cache)?;
    let artefacts = Artefacts::new(&config, &config_root, sources.images());
    if opts.resolved {
        if opts.major || opts.create.is_some() {
            bail!("--resolved writes what resolution selected: it takes neither --major nor -c");
        }
        return resolved(
            &config,
            &config_root,
            &sources,
            &artefacts,
            opts,
            verbose,
            out,
        );
    }
    if sources.stores.is_none() {
        bail!(
            "upgrade edits configs in the checkouts of a developer machine; CI keeps none to \
             edit"
        );
    }
    let resolution = crate::resolve::workspace(
        &config,
        &config_root,
        true,
        &sources,
        Some(&artefacts),
        verbose,
    )?;
    if opts.dirs.is_empty() {
        if opts.major {
            bail!(
                "--major raises a named dependency: gitscale upgrade --major <dir>. Promotion \
                 stays in the major each pin is in"
            );
        }
        if opts.create.is_some() {
            bail!("-c names the topic an upgrade of named dependencies creates");
        }
        promote_topic(
            &config,
            &config_root,
            &sources,
            &artefacts,
            &resolution,
            opts,
            out,
        )
    } else {
        raise(&config, &config_root, &resolution, opts, verbose, out)
    }
}

// ---------------------------------------------------------------------------
// Promotion
// ---------------------------------------------------------------------------

fn promote_topic(
    config: &GitScaleConfig,
    config_root: &Path,
    sources: &Sources,
    artefacts: &Artefacts,
    resolution: &Resolution,
    opts: &Options,
    out: &mut dyn Write,
) -> Result<()> {
    let Some(branch) = crate::topic::root(config, config_root, true)
        .topic()
        .map(str::to_string)
    else {
        bail!(
            "not on a topic: promotion raises the pins of a topic's repositories once they are \
             released. Name what to raise — gitscale upgrade <dir> — or write what resolution \
             selected with gitscale upgrade --resolved"
        );
    };
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
        let has_image = move |commit: &str| artefacts.has_image(&url, commit);
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

    print_edits(&edits, out)?;
    if !opts.dry_run {
        let placer = Placer {
            config_root,
            sources,
            artefacts,
            verbose: false,
        };
        for (slot, tag) in &leaving {
            let left = leave_topic(config_root, resolution, &placer, slot, tag)?;
            writeln!(out, "  {}  left the topic: {}", slot.directory, left)?;
            if crate::commands::develop::remote_has_branch(&slot.url, &branch) {
                writeln!(
                    out,
                    "  {}: branch {} still exists on its remote; pull and CI keep matching it by \
                     name until it is deleted",
                    slot.directory, branch
                )?;
            }
        }
        apply(&edits, opts.commit, out)?;
    }
    let next = promote::next_to_merge(&requests, &promoted);
    if !next.is_empty() {
        writeln!(out, "next to merge: {}", next.join(", "))?;
    }
    if opts.dry_run {
        writeln!(out, "(dry run: nothing changed)")?;
    }
    Ok(())
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
    let mut off = slot.clone();
    off.topic = None;
    off.commit = Some(commit);
    let placed = placer.place(&off)?;
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

fn raise(
    config: &GitScaleConfig,
    config_root: &Path,
    resolution: &Resolution,
    opts: &Options,
    verbose: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let current = crate::topic::root(config, config_root, true)
        .topic()
        .map(str::to_string);
    if let (Some(current), Some(asked)) = (&current, opts.create) {
        if current != asked {
            bail!(
                "already on topic {}; -c names a topic to create from the default branch",
                current
            );
        }
    }
    let mut edits = Vec::new();
    let mut joiners: Vec<&Slot> = Vec::new();
    let mut raised = Vec::new();
    for dir in opts.dirs {
        let slot = resolution
            .find(dir)
            .ok_or_else(|| anyhow::anyhow!("no checkout at {}", dir))?;
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
        let current = match (&slot.topic, &slot.chosen) {
            (Some(topic), _) => topic.pin.as_ref().map(|p| p.revision.clone()),
            (None, Some(chosen)) => Some(chosen.revision.clone()),
            _ => None,
        };
        let Some(current) = current.filter(|c| version::parse(c).is_some()) else {
            writeln!(
                out,
                "{} is not pinned to a version; nothing to raise from",
                slot.directory
            )?;
            continue;
        };
        let pinned = version::parse(&current).expect("checked above");
        let tags = release_tags(&slot.url)?;
        let Some((tag, newest)) =
            promote::newest(tags.iter().map(String::as_str), &pinned, opts.major)
        else {
            writeln!(
                out,
                "{} has no release to raise {} to",
                slot.directory, current
            )?;
            continue;
        };
        if pinned.compare(&newest) != Some(std::cmp::Ordering::Less) {
            writeln!(
                out,
                "{} is already at its newest release, {}",
                slot.directory, current
            )?;
            continue;
        }
        writeln!(out, "{}   {} → {}", slot.directory, current, tag)?;
        raised.push(slot.directory.clone());
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
                if !on_its_branch(config_root, requester)
                    && !joiners.iter().any(|j| j.directory == requester.directory)
                {
                    joiners.push(requester);
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

    if !joiners.is_empty() {
        let branch = match (&current, opts.create) {
            (Some(current), _) => current.clone(),
            (None, Some(asked)) => asked.to_string(),
            (None, None) => generated_branch(&raised, &edits),
        };
        let names: Vec<String> = joiners.iter().map(|j| j.directory.clone()).collect();
        if opts.dry_run {
            writeln!(
                out,
                "  would develop on topic {}: {}",
                branch,
                names.join(", ")
            )?;
        } else {
            // No topic yet: the root's new branch is the topic.
            if current.is_none() {
                crate::git::run_git(
                    &["switch", "--quiet", "-c", &branch],
                    Some(config_root),
                    true,
                )
                .with_context(|| format!("cannot create branch {} in the root", branch))?;
            }
            crate::commands::develop::run(
                Some(config_root),
                &names,
                false,
                verbose,
                &mut std::io::sink(),
            )?;
            let created = if current.is_none() { " (created)" } else { "" };
            writeln!(
                out,
                "  topic {}{}: {} developed",
                branch,
                created,
                names.join(", ")
            )?;
        }
    }

    print_edits(&edits, out)?;
    if opts.dry_run {
        writeln!(out, "(dry run: nothing changed)")?;
        return Ok(());
    }
    apply(&edits, opts.commit, out)?;
    let mut next: Vec<String> = Vec::new();
    for edit in &edits {
        let dir = edit
            .repo
            .strip_prefix(config_root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let dir = if dir.is_empty() { ".".to_string() } else { dir };
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
    Ok(())
}

/// Every release tag of the repository at `url`, from its remote.
fn release_tags(url: &str) -> Result<Vec<String>> {
    let (refs, _) = crate::git::ls_remote_full(&crate::ci::remote_url(url))
        .with_context(|| format!("cannot list the tags of {}", url))?;
    Ok(refs
        .into_iter()
        .filter_map(|(_, name)| name.strip_prefix("refs/tags/").map(str::to_string))
        .collect())
}

/// The topic an upgrade creates when there is none: named after the one
/// dependency and its new tag, or the day for several.
fn generated_branch(raised: &[String], edits: &[Edit]) -> String {
    match raised {
        [one] => {
            let name = Path::new(one)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| one.clone());
            let tag = edits
                .iter()
                .find(|e| &e.dependency == one)
                .map(|e| e.to.clone())
                .unwrap_or_default();
            format!("upgrade/{}-{}", name, tag)
        }
        _ => format!("upgrade/{}", chrono::Local::now().format("%Y-%m-%d")),
    }
}

// ---------------------------------------------------------------------------
// Writing what resolution selected
// ---------------------------------------------------------------------------

fn resolved(
    config: &GitScaleConfig,
    config_root: &Path,
    sources: &Sources,
    artefacts: &Artefacts,
    opts: &Options,
    verbose: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let resolution =
        crate::resolve::workspace(config, config_root, true, sources, Some(artefacts), verbose)?;
    for dir in opts.dirs {
        if !config.repos.iter().any(|e| &e.directory == dir) {
            bail!("{} is not an entry of the root {}", dir, CONFIG_FILENAME);
        }
    }

    // Only entries that already give a revision: one without leaves it to
    // the dependencies on purpose, and an override is the root's own choice.
    struct Change {
        directory: String,
        from: String,
        to: String,
        by: String,
        branch: bool,
    }
    let mut changes = Vec::new();
    for entry in &config.repos {
        if entry.revision.is_empty()
            || entry.is_override
            || (!opts.dirs.is_empty() && !opts.dirs.contains(&entry.directory))
        {
            continue;
        }
        let Some(slot) = resolution.slot(&entry.directory) else {
            continue;
        };
        // On a topic: what a merge would pin, never the topic branch.
        let (to, by, kind) = match (&slot.topic, &slot.chosen) {
            (Some(topic), _) => match &topic.pin {
                Some(pin) => (pin.revision.clone(), pin.by.clone(), None),
                None => continue,
            },
            (None, Some(chosen)) => (
                chosen.revision.clone(),
                chosen.by.clone(),
                Some(chosen.kind),
            ),
            _ => continue,
        };
        if to == entry.revision {
            continue;
        }
        changes.push(Change {
            directory: entry.directory.clone(),
            from: entry.revision.clone(),
            to,
            by,
            branch: kind == Some(crate::resolution::RevKind::Branch),
        });
    }

    if changes.is_empty() {
        writeln!(
            out,
            "Every root entry already declares the revision it resolves to."
        )?;
        return Ok(());
    }
    let width = |f: &dyn Fn(&Change) -> &str| {
        changes
            .iter()
            .map(|c| f(c).chars().count())
            .max()
            .unwrap_or(0)
    };
    let (dw, fw, tw) = (
        width(&|c| &c.directory),
        width(&|c| &c.from),
        width(&|c| &c.to),
    );
    for change in &changes {
        let moving = if change.branch {
            ", a branch: the pin will move"
        } else {
            ""
        };
        writeln!(
            out,
            "{:<dw$}   {:<fw$} → {:<tw$}   raised by {}{}",
            change.directory,
            change.from,
            change.to,
            change.by,
            moving,
            dw = dw,
            fw = fw,
            tw = tw,
        )?;
    }
    let count = crate::cache::plural(changes.len(), "entry", "entries");
    if opts.dry_run {
        writeln!(out, "{} would change (dry run: nothing changed)", count)?;
        return Ok(());
    }
    let edits: Vec<Edit> = changes
        .iter()
        .map(|c| Edit {
            repo: config_root.to_path_buf(),
            label: CONFIG_FILENAME.to_string(),
            key: c.directory.clone(),
            from: c.from.clone(),
            to: c.to.clone(),
            dependency: c.directory.clone(),
        })
        .collect();
    set_revisions(
        &config_root.join(CONFIG_FILENAME),
        &edits
            .iter()
            .map(|e| (e.key.clone(), e.to.clone()))
            .collect::<Vec<_>>(),
    )?;
    writeln!(out, "Updated {} in {}", count, CONFIG_FILENAME)?;
    if opts.commit {
        commit_configs(&edits, out)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared
// ---------------------------------------------------------------------------

/// Whether `entry` asks for the repository `slot` checks out.
fn same_repo(entry: &RepoEntry, slot: &Slot) -> bool {
    crate::urls::normalize(&entry.repo_url) == crate::urls::normalize(&slot.url)
}

/// Whether `revision` is a version below `tag`: `None` when the two are not
/// versions of one stream, so nothing can be said.
fn below(revision: &str, tag: &str) -> Option<bool> {
    let (have, want) = (version::parse(revision)?, version::parse(tag)?);
    have.compare(&want)
        .map(|order| order == std::cmp::Ordering::Less)
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
fn apply(edits: &[Edit], commit: bool, out: &mut dyn Write) -> Result<()> {
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
        commit_configs(edits, out)?;
    }
    Ok(())
}

/// Commit each edited `.gitscale.toml` — that file alone, whatever else the
/// repository has changed — as `pin <dependency> <tag>`.
fn commit_configs(edits: &[Edit], out: &mut dyn Write) -> Result<()> {
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
    for (repo, (label, items)) in by_repo {
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
    Ok(())
}

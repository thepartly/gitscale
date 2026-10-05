//! Putting each checkout where resolution says it goes.
//!
//! On a developer machine every git checkout is a worktree of the root's
//! store for its repository (see [`crate::store`]): detached at the commit
//! resolution selected and read-only, or — on the workspace's topic — on the
//! topic branch and writable. In CI it is a depth-1 copy of exactly that
//! commit, from the per-user cache when there is one, and detached and
//! read-only likewise: what a workspace builds and what a pipeline builds are
//! the same files.
//!
//! An artefact entry that replaces its checkout holds the image of its
//! commit, unless it is on the topic: developed here, or following a remote
//! branch whose tip has no image yet, it is the source instead.
//!
//! Nothing is moved that could lose anything: uncommitted changes, or commits
//! no branch, tag or remote holds, fail the entry and leave it where it is.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

use crate::artefact::{Artefacts, Pulled};
use crate::config::RepoEntry;
use crate::resolution::Slot;
use crate::store::{Sources, Worktree};

/// Where a git checkout goes.
#[derive(Debug, Clone)]
enum Target {
    /// Detached at a commit, read-only.
    Detached(String),
    /// On a topic branch, writable: the store's own when `developed`, else
    /// one made to track the remote's.
    Branch {
        name: String,
        commit: String,
        developed: bool,
    },
}

impl Target {
    fn commit(&self) -> &str {
        match self {
            Target::Detached(commit) => commit,
            Target::Branch { commit, .. } => commit,
        }
    }
}

/// One command's means of placing checkouts.
pub struct Placer<'a> {
    pub config_root: &'a Path,
    pub sources: &'a Sources,
    pub artefacts: &'a Artefacts,
    pub verbose: bool,
    /// Fetch each store before placing from it. Off, a store is fetched
    /// only when it lacks the commit to place.
    pub fetch: bool,
}

impl Placer<'_> {
    /// Put `slot`'s checkout where resolution says, cloning it first when it
    /// is not there. The message says what it holds now.
    pub fn place(&self, slot: &Slot) -> Result<String> {
        let entry = slot.entry();
        let name = slot.directory.as_str();
        let Some(commit) = slot.commit.clone() else {
            bail!(
                "{} has no revision to check out{}",
                name,
                slot.unresolved
                    .as_ref()
                    .map(|r| format!(": {}", r))
                    .unwrap_or_default()
            );
        };
        if entry.is_artefact() {
            return self.place_artefact(slot, &entry, &commit);
        }
        let target = match &slot.topic {
            Some(topic) => Target::Branch {
                name: topic.branch.clone(),
                commit: topic.commit.clone(),
                developed: topic.developed,
            },
            None => Target::Detached(commit),
        };
        let mut message = self.place_source(&entry, &target)?;
        if entry.is_overlay() {
            message.push_str(&self.overlay(slot, &entry)?);
        }
        Ok(message)
    }

    /// An artefact entry: the image, or — on the topic, without one to use —
    /// the source.
    fn place_artefact(&self, slot: &Slot, entry: &RepoEntry, commit: &str) -> Result<String> {
        let dest = self.config_root.join(&entry.directory);
        let name = entry.directory.as_str();
        if let Some(topic) = &slot.topic {
            let sources_needed = topic.developed
                || !self
                    .artefacts
                    .has_image(&entry.repo_url, &topic.commit)
                    .with_context(|| format!("cannot ask the registry about {}", name))?;
            if sources_needed {
                if !crate::git::is_checkout(&dest) {
                    crate::artefact::uninstall(self.config_root, name)?;
                }
                // Developed: on its branch. Following a branch whose tip has
                // no image: the source of that tip, detached — the image takes
                // over again once one is published.
                let target = if topic.developed {
                    Target::Branch {
                        name: topic.branch.clone(),
                        commit: topic.commit.clone(),
                        developed: true,
                    }
                } else {
                    Target::Detached(topic.commit.clone())
                };
                let mut source = entry.clone();
                source.artefact = None;
                let placed = self.place_source(&source, &target)?;
                return Ok(format!("{}, sources", placed));
            }
        }
        if crate::git::is_checkout(&dest) {
            self.remove_source(entry, &dest)?;
        }
        let commit = slot.topic.as_ref().map_or(commit, |t| t.commit.as_str());
        Ok(match self.artefacts.pull(entry, &dest, commit)? {
            Pulled::Updated(commit) => {
                format!("{} (artefact {})", name, crate::git::short_sha(&commit))
            }
            Pulled::Current(commit) => format!(
                "{} (artefact {}, up to date)",
                name,
                crate::git::short_sha(&commit)
            ),
        })
    }

    /// A source checkout an artefact entry is done with — it left the topic,
    /// or its branch's tip has an image now — taken away so the image can go
    /// in its place. Refused while it holds work.
    fn remove_source(&self, entry: &RepoEntry, dest: &Path) -> Result<()> {
        if let Some(work) = crate::git::local_work(dest, &[]) {
            bail!(
                "not replaced by its artefact: {}; commit and push it, or discard it, then pull \
                 again",
                work
            );
        }
        crate::git::restore_writable(dest)?;
        match &self.sources.stores {
            Some(stores) => {
                let store = stores.repo_path(&crate::git::remote_url(entry));
                let dest_str = dest.to_string_lossy().to_string();
                crate::git::run_git(
                    &["worktree", "remove", "--force", &dest_str],
                    Some(&store),
                    true,
                )?;
            }
            None => std::fs::remove_dir_all(dest)
                .with_context(|| format!("cannot remove {}", dest.display()))?,
        }
        Ok(())
    }

    /// The overlay of an overlay entry, for the commit its checkout is at.
    /// Off the topic it must exist; on it, a commit without an image just has
    /// none laid over it.
    fn overlay(&self, slot: &Slot, entry: &RepoEntry) -> Result<String> {
        let dest = self.config_root.join(&entry.directory);
        let head = crate::git::resolve_ref(&dest, "HEAD")
            .ok_or_else(|| anyhow::anyhow!("HEAD does not resolve in {}", entry.directory))?;
        let on_branch = crate::git::current_branch(&dest).is_some();
        if slot.topic.is_some() && !self.artefacts.has_image(&entry.repo_url, &head)? {
            return Ok(String::new());
        }
        Ok(
            match self.artefacts.overlay(entry, &dest, &head, on_branch)? {
                Pulled::Updated(commit) => format!(" (overlay {})", crate::git::short_sha(&commit)),
                Pulled::Current(_) => String::new(),
            },
        )
    }

    /// A git checkout at `target`.
    fn place_source(&self, entry: &RepoEntry, target: &Target) -> Result<String> {
        let dest = self.config_root.join(&entry.directory);
        let name = entry.directory.as_str();
        if dest.exists() && !crate::git::is_checkout(&dest) && !crate::artefact::is_empty_dir(&dest)
        {
            bail!(
                "{} exists but holds no git repository; `git scale clean -fd {}` removes it",
                dest.display(),
                name
            );
        }
        let mut notes = Vec::new();
        match &self.sources.stores {
            Some(stores) => {
                let url = crate::git::remote_url(entry);
                let store = if self.fetch {
                    stores.update(&url)?
                } else {
                    stores.ready(&url, target.commit())?
                };
                if crate::git::is_checkout(&dest) {
                    match crate::store::identify(&store, &dest) {
                        Worktree::Ours => {}
                        Worktree::Repaired => notes.push("repaired after a move".to_string()),
                        Worktree::Foreign => bail!(
                            "not a gitscale worktree; move your changes out, delete it and run \
                             git scale sync"
                        ),
                    }
                } else {
                    crate::store::add_worktree(&store, &dest, target.commit())?;
                    if matches!(target, Target::Detached(_)) {
                        crate::git::apply_readonly(&dest)?;
                    }
                }
                move_to(&dest, target)?;
            }
            None => {
                let commit = target.commit();
                if crate::git::is_checkout(&dest) {
                    self.ci_refresh(entry, &dest, commit)?;
                } else {
                    self.ci_clone(entry, &dest, commit)?;
                }
                crate::git::apply_readonly(&dest)?;
            }
        }
        if let Target::Branch { name: branch, .. } = target {
            notes.insert(0, format!("on {}", branch));
        }
        Ok(if notes.is_empty() {
            name.to_string()
        } else {
            format!("{} ({})", name, notes.join(", "))
        })
    }

    /// CI: a new checkout of `commit` at depth 1 — a local clone of the
    /// cache's snapshot pin when there is a cache, otherwise one commit
    /// fetched from the remote.
    fn ci_clone(&self, entry: &RepoEntry, dest: &Path, commit: &str) -> Result<()> {
        let progress = if self.verbose {
            "--progress"
        } else {
            "--quiet"
        };
        if let Some(cache) = &self.sources.cache {
            if let Some(pinned) = cache.pin(&crate::git::remote_url(entry), commit)? {
                let from = pinned.entry.to_string_lossy().to_string();
                let dest_str = dest.to_string_lossy().to_string();
                if dest.is_dir() {
                    std::fs::remove_dir(dest)
                        .with_context(|| format!("cannot remove {}", dest.display()))?;
                }
                crate::git::run_git(
                    &[
                        "clone",
                        // Without this the job clones every other pin too.
                        "--single-branch",
                        "--branch",
                        &pinned.reference,
                        "--no-checkout",
                        progress,
                        &from,
                        &dest_str,
                    ],
                    None,
                    true,
                )?;
                // `origin` names the cache entry; the checkout's remote has to
                // be the real one, for anything run in it by hand.
                crate::git::reconcile_remote(entry, self.config_root)?;
                crate::git::run_git(
                    &["checkout", "--quiet", "--detach", &pinned.sha],
                    Some(dest),
                    true,
                )?;
                crate::git::run_git(&["branch", "-D", &pinned.reference], Some(dest), false)?;
                return Ok(());
            }
        }
        std::fs::create_dir_all(dest)
            .with_context(|| format!("cannot create {}", dest.display()))?;
        crate::git::run_git(&["init", "--quiet"], Some(dest), true)?;
        crate::git::run_git(
            &["remote", "add", "origin", &crate::git::remote_url(entry)],
            Some(dest),
            true,
        )?;
        fetch_commit(entry, dest, commit)?;
        crate::git::run_git(
            &["checkout", "--quiet", "--detach", commit],
            Some(dest),
            true,
        )?;
        Ok(())
    }

    /// CI: an existing checkout moved to `commit`, fetched at depth 1. A CI
    /// checkout holds nobody's work, so the move is forced.
    fn ci_refresh(&self, entry: &RepoEntry, dest: &Path, commit: &str) -> Result<()> {
        if crate::git::resolve_ref(dest, "HEAD").as_deref() == Some(commit) {
            return Ok(());
        }
        crate::git::ensure_ci_remote(entry, dest)?;
        let have = crate::git::ref_exists(dest, &format!("{}^{{commit}}", commit));
        if !have {
            let pinned = match &self.sources.cache {
                Some(cache) => cache.pin(&crate::git::remote_url(entry), commit)?,
                None => None,
            };
            match pinned {
                Some(pinned) => {
                    let from = pinned.entry.to_string_lossy().to_string();
                    crate::git::run_git(
                        &["fetch", "--depth", "1", "--quiet", &from, &pinned.reference],
                        Some(dest),
                        true,
                    )?;
                }
                None => fetch_commit(entry, dest, commit)?,
            }
        }
        crate::git::restore_writable(dest)?;
        crate::git::run_git(
            &["checkout", "--quiet", "-f", "--detach", commit],
            Some(dest),
            true,
        )?;
        Ok(())
    }
}

/// Bring `commit` into the checkout at `dest` at depth 1: by its SHA, and —
/// for a remote that will not serve a bare commit — by the revision that
/// names it.
fn fetch_commit(entry: &RepoEntry, dest: &Path, commit: &str) -> Result<()> {
    let fetch = |what: &str| {
        crate::git::run_git(
            &["fetch", "--depth", "1", "--quiet", "origin", what],
            Some(dest),
            false,
        )
        .is_ok_and(|o| o.status.success())
    };
    let have = || crate::git::ref_exists(dest, &format!("{}^{{commit}}", commit));
    if fetch(commit) && have() {
        return Ok(());
    }
    if !entry.revision.is_empty()
        && !crate::git::is_full_sha(&entry.revision)
        && fetch(&entry.revision)
        && have()
    {
        return Ok(());
    }
    bail!(
        "cannot fetch {} of {}",
        crate::git::short_sha(commit),
        entry.repo_url
    )
}

/// Move the worktree at `dest` to `target`, if it is not there already.
fn move_to(dest: &Path, target: &Target) -> Result<()> {
    let branch = crate::git::current_branch(dest);
    let head = crate::git::resolve_ref(dest, "HEAD");
    match target {
        Target::Detached(commit) => {
            if branch.is_none() && head.as_deref() == Some(commit.as_str()) {
                return Ok(());
            }
            // Off a branch: its commits stay on it, so nothing is lost there;
            // a detached HEAD's own commits are checked.
            crate::git::ensure_switchable(dest, None, false)?;
            crate::git::restore_writable(dest)?;
            let moved = crate::git::move_checkout(dest, &["--detach", commit]);
            crate::git::apply_readonly(dest)?;
            moved
        }
        Target::Branch {
            name, developed, ..
        } => {
            if branch.as_deref() != Some(name.as_str()) {
                // Onto the topic: uncommitted edits come along, as with
                // `git checkout`, which refuses one that conflicts.
                crate::git::ensure_switchable(dest, None, true)?;
                crate::git::restore_writable(dest)?;
                if *developed {
                    crate::git::move_checkout(dest, &[name])?;
                } else {
                    let tracking = format!("origin/{}", name);
                    crate::git::move_checkout(dest, &["-b", name, "--track", &tracking])?;
                }
            }
            crate::git::fast_forward(dest)
        }
    }
}

/// Put the checkout at `dest`, detached at its pin, on a new branch `name`
/// at the commit it is at — what `git topic join` does — and make it
/// writable.
pub fn start_branch(dest: &Path, name: &str) -> Result<()> {
    crate::git::restore_writable(dest)?;
    crate::git::run_git(&["switch", "--quiet", "-c", name], Some(dest), true)
        .with_context(|| format!("cannot create branch {}", name))?;
    Ok(())
}

/// Carry the children on topic `from` to `to`, the branch the root was just
/// created on from it: each worktree of this root's stores on `from` (or
/// `from@v<major>`) gets the same branch of `to` at its current commit,
/// uncommitted edits included. Returns the checkouts carried.
pub fn carry(sources: &Sources, config_root: &Path, from: &str, to: &str) -> Vec<String> {
    let Some(stores) = &sources.stores else {
        return Vec::new();
    };
    let root = config_root
        .canonicalize()
        .unwrap_or_else(|_| config_root.to_path_buf());
    let mut carried = Vec::new();
    for store in stores.all() {
        for (path, branch) in worktrees(&store) {
            if !path.starts_with(&root) || path == root {
                continue;
            }
            let suffix = match branch.strip_prefix(from) {
                Some(rest) if rest.is_empty() || rest.starts_with("@v") => rest.to_string(),
                _ => continue,
            };
            let new = format!("{}{}", to, suffix);
            if crate::git::ref_exists(&store, &format!("refs/heads/{}", new)) {
                continue;
            }
            let switched =
                crate::git::run_git(&["switch", "--quiet", "-c", &new], Some(&path), true);
            if switched.is_ok() {
                carried.push(
                    path.strip_prefix(&root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    carried
}

/// Carry topic `from` to `to` in the stores themselves, for a topic started
/// from one the root is not on: each store with a branch `from` (or
/// `from@v<major>`) gets the same branch of `to` at the same commit, which
/// placement then puts its checkout on. Returns how many were made.
pub fn carry_branches(sources: &Sources, from: &str, to: &str) -> usize {
    let Some(stores) = &sources.stores else {
        return 0;
    };
    let mut made = 0;
    for store in stores.all() {
        let listed = crate::git::query(
            &store,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
        )
        .unwrap_or_default();
        for branch in listed.lines() {
            let suffix = match branch.strip_prefix(from) {
                Some(rest) if rest.is_empty() || rest.starts_with("@v") => rest,
                _ => continue,
            };
            let new = format!("{}{}", to, suffix);
            if crate::git::ref_exists(&store, &format!("refs/heads/{}", new)) {
                continue;
            }
            let made_one = crate::git::run_git(
                &[
                    "branch",
                    "--quiet",
                    "--no-track",
                    &new,
                    &format!("refs/heads/{}", branch),
                ],
                Some(&store),
                true,
            );
            if made_one.is_ok() {
                made += 1;
            }
        }
    }
    made
}

/// Each worktree of `store` that is on a branch: its path and the branch.
pub fn worktrees(store: &Path) -> Vec<(PathBuf, String)> {
    let Some(listing) = crate::git::query(store, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut path: Option<PathBuf> = None;
    for line in listing.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(p));
        } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
            if let Some(p) = &path {
                found.push((
                    p.canonicalize().unwrap_or_else(|_| p.clone()),
                    branch.to_string(),
                ));
            }
        }
    }
    found
}

/// What `git topic leave` and promotion do to a joined checkout:
/// detached at `commit`, read-only again, its branch `branch` deleted from
/// the store. Refused while the checkout holds work nothing else has —
/// `planted` are the links gitscale put there, which are not. `released`
/// says a tag already holds the branch's content: commits no remote has are
/// then what a squash left behind, not work.
pub fn stop_branch(
    dest: &Path,
    branch: &str,
    commit: &str,
    planted: &[String],
    released: bool,
) -> Result<()> {
    let work = if released {
        crate::git::uncommitted(dest, planted).map(|n| {
            format!(
                "{} uncommitted",
                crate::cache::plural(n, "change", "changes")
            )
        })
    } else {
        crate::git::local_work(dest, planted)
    };
    if let Some(work) = work {
        bail!("it has {}; commit and push it, or discard it, first", work);
    }
    crate::git::restore_writable(dest)?;
    crate::git::run_git(
        &["checkout", "--quiet", "--detach", commit],
        Some(dest),
        true,
    )?;
    crate::git::apply_readonly(dest)?;
    // Everything on it is pushed, or released — checked above — so nothing
    // is lost however the branch was merged.
    crate::git::run_git(&["branch", "--quiet", "-D", branch], Some(dest), true)?;
    Ok(())
}

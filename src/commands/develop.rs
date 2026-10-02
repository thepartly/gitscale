//! `gitscale develop <dir>`: put a checkout on the workspace's topic — the
//! root's current branch — starting from the commit it is detached at, and
//! make it writable. `--stop` takes it back off.
//!
//! Nothing else is needed to work across repositories: `git switch` on the
//! root is how a topic is entered, left and changed, and every `pull` places
//! each checkout by the topic's branches.

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::Path;

use crate::artefact::Artefacts;
use crate::checkout::Placer;
use crate::config::{load_workspace, CONFIG_FILENAME};
use crate::resolution::Resolution;
use crate::store::Sources;
use crate::topic::Root;

pub fn run(
    root: Option<&Path>,
    dirs: &[String],
    stop: bool,
    verbose: bool,
    out: &mut dyn Write,
) -> Result<()> {
    if dirs.is_empty() {
        bail!("name the checkout to develop: gitscale develop <dir>");
    }
    let (config, config_root) = load_workspace(root)?;
    let sources = Sources::new(&config_root, false)?;
    let Some(stores) = &sources.stores else {
        bail!("develop works on a developer machine: CI checkouts are copies of exact commits");
    };
    stores.tidy(&config_root);
    let branch = match crate::topic::root(&config, &config_root, true) {
        Root::Topic(branch) => branch,
        Root::Pinned(branch) => bail!(
            "the root is on {}, which it pins: its checkouts stay at their pins. Create a \
             branch first: git switch -c <topic>",
            branch
        ),
        Root::Detached => {
            bail!("the root is on no branch. Create one first: git switch -c <topic>")
        }
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
    let placer = Placer {
        config_root: &config_root,
        sources: &sources,
        artefacts: &artefacts,
        verbose,
    };
    let mut failed = 0;
    for dir in dirs {
        let done = if stop {
            stop_one(&config_root, &resolution, &placer, dir)
        } else {
            start_one(&config, &config_root, &resolution, &placer, &branch, dir)
        };
        match done {
            Ok(message) => writeln!(out, "{}", message)?,
            Err(e) => {
                failed += 1;
                writeln!(out, "FAIL  {}: {:#}", dir, e)?;
            }
        }
    }
    if failed > 0 {
        bail!(
            "{} not {}",
            crate::cache::plural(failed, "checkout", "checkouts"),
            if stop {
                "taken off the topic"
            } else {
                "developed"
            }
        );
    }
    Ok(())
}

/// Put one checkout on the topic.
fn start_one(
    config: &crate::config::GitScaleConfig,
    config_root: &Path,
    resolution: &Resolution,
    placer: &Placer,
    topic: &str,
    dir: &str,
) -> Result<String> {
    let slot = resolution
        .find(dir)
        .ok_or_else(|| anyhow::anyhow!("no checkout at {}", dir))?;
    let name = slot.directory.as_str();
    if config
        .repos
        .iter()
        .any(|e| e.directory == slot.directory && e.is_override)
    {
        bail!(
            "{} is held by override in {}; remove the override first",
            name,
            CONFIG_FILENAME
        );
    }
    if let Some(by) = &slot.pinned_by {
        bail!(
            "{} pins {} for its dependencies, so {} stays at its pin",
            by,
            topic,
            name
        );
    }
    let branch = slot
        .branch
        .clone()
        .ok_or_else(|| anyhow::anyhow!("{} cannot be on topic {}", name, topic))?;
    if let Some(on) = &slot.topic {
        if on.developed {
            return Ok(format!("{} is already on {}", name, branch));
        }
    }
    let dest = config_root.join(name);
    let entry = slot.entry();
    // Developing an artefact means its source: the image goes, a worktree of
    // the same commit comes.
    let from = match &slot.topic {
        Some(on) => on.commit.clone(),
        None if entry.is_artefact() => crate::artefact::installed_commit(config_root, name)
            .or_else(|| slot.commit.clone())
            .ok_or_else(|| anyhow::anyhow!("{} has no commit to start from", name))?,
        None => match crate::git::resolve_ref(&dest, "HEAD") {
            Some(head) if crate::git::is_checkout(&dest) => head,
            _ => slot
                .commit
                .clone()
                .ok_or_else(|| anyhow::anyhow!("{} has no commit to start from", name))?,
        },
    };
    let stores = placer
        .sources
        .stores
        .as_ref()
        .expect("develop runs with stores");
    let store = stores.update(&crate::git::remote_url(&entry))?;
    if entry.is_artefact() && !crate::git::is_checkout(&dest) {
        crate::artefact::uninstall(config_root, name)?;
    }
    if !crate::git::is_checkout(&dest) {
        crate::store::add_worktree(&store, &dest, &from)?;
    } else if crate::store::identify(&store, &dest) == crate::store::Worktree::Foreign {
        bail!("not a gitscale worktree; move your changes out, delete it and run pull");
    }
    match &slot.topic {
        // Its remote has the branch already: the store's own copy of it,
        // tracking the remote's, is where the work goes on.
        Some(_) => {
            crate::git::restore_writable(&dest)?;
            crate::git::move_checkout(
                &dest,
                &["-b", &branch, "--track", &format!("origin/{}", branch)],
            )
            .with_context(|| format!("cannot put {} on {}", name, branch))?;
        }
        None => crate::checkout::start_branch(&dest, &branch)?,
    }
    let pin = slot
        .pin()
        .map(|p| format!("{} ({})", p.revision, crate::git::short_sha(&p.commit)))
        .unwrap_or_else(|| crate::git::short_sha(&from).to_string());
    Ok(format!("{} on {}, from {}", name, branch, pin))
}

/// Take one checkout off the topic: back at its pin, its branch deleted.
fn stop_one(
    config_root: &Path,
    resolution: &Resolution,
    placer: &Placer,
    dir: &str,
) -> Result<String> {
    let slot = resolution
        .find(dir)
        .ok_or_else(|| anyhow::anyhow!("no checkout at {}", dir))?;
    let name = slot.directory.as_str();
    let Some(on) = &slot.topic else {
        bail!("{} is not on the topic", name);
    };
    let dest = config_root.join(name);
    if remote_has_branch(&slot.url, &on.branch) {
        bail!(
            "{} exists on {}: pull follows it as long as it does. Delete it there first, then \
             run gitscale develop --stop {} again",
            on.branch,
            slot.url,
            name
        );
    }
    let pin = match slot.pin() {
        Some(pin) => pin.commit,
        // Nobody gives it a revision: the default branch is its pin.
        None => crate::git::resolve_ref(&dest, "refs/remotes/origin/HEAD")
            .ok_or_else(|| anyhow::anyhow!("{} has no pin to go back to", name))?,
    };
    crate::checkout::stop_branch(&dest, &on.branch, &pin, &resolution.planted_in(name), false)
        .with_context(|| format!("{} stays on {}", name, on.branch))?;
    // Back to what it is off the topic — for an artefact, its image.
    let mut off = slot.clone();
    off.topic = None;
    off.commit = Some(pin);
    let placed = placer.place(&off)?;
    Ok(format!("{} left {}: {}", name, on.branch, placed))
}

/// Whether the remote at `url` has a branch `branch`.
pub fn remote_has_branch(url: &str, branch: &str) -> bool {
    let url = crate::ci::remote_url(url);
    matches!(
        crate::git::ls_remote_revision(&url, &format!("refs/heads/{}", branch), false),
        Ok(Some(_))
    )
}

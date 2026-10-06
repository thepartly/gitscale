//! What `git scale fetch` does after git's own fetch in each repository:
//! refresh every store of the root, and what the registry has for each
//! artefact's release. Nothing is placed: the next placement uses what this
//! brought.

use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::artefact::Artefacts;
use crate::config::GitScaleConfig;
use crate::progress::{run_parallel, RepoStatus};
use crate::store::Sources;

pub fn refresh(
    config: &GitScaleConfig,
    config_root: &Path,
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let sources = Sources::new(config_root, no_cache)?;
    let artefacts = Artefacts::new(config, config_root, sources.images());
    // Resolving online is most of the fetch: every store resolution reads is
    // brought up to date on the way, implicit dependencies included. A graph
    // that will not resolve — a remote that cannot be reached, a conflict —
    // still gets every store fetched, and fails afterwards with the reason.
    let resolved = crate::resolve::workspace(
        config,
        config_root,
        true,
        &sources,
        Some(&artefacts),
        verbose,
    );
    // One job per checkout taken as an artefact, and per other store, each
    // named as the workspace knows it: a store by the checkouts made from
    // it, else by its own name.
    enum Job {
        /// The entry, and the commit its release is.
        Artefact(crate::config::RepoEntry, String),
        Store(std::path::PathBuf),
    }
    let mut jobs: Vec<(String, Job)> = Vec::new();
    let slots = resolved
        .as_ref()
        .map(|r| r.slots.as_slice())
        .unwrap_or_default();
    for slot in slots {
        let entry = slot.entry();
        let image = slot.form() == crate::prefer::Form::Artefact;
        if image && !config_root.join(&entry.directory).is_symlink() {
            let commit = slot.commit.clone().unwrap_or_default();
            jobs.push((entry.directory.clone(), Job::Artefact(entry, commit)));
        }
    }
    let imaged = |path: &std::path::Path| {
        slots.iter().any(|s| {
            s.form() == crate::prefer::Form::Artefact
                && sources
                    .stores
                    .as_ref()
                    .is_some_and(|st| st.repo_path(&crate::ci::remote_url(&s.url)) == path)
        })
    };
    if let Some(stores) = &sources.stores {
        for path in stores.all().into_iter().filter(|p| !imaged(p)) {
            let label = slots
                .iter()
                .filter(|s| s.form() == crate::prefer::Form::Source)
                .find(|s| stores.repo_path(&crate::ci::remote_url(&s.url)) == path)
                .map(|s| s.directory.clone())
                .unwrap_or_else(|| {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                });
            jobs.push((label, Job::Store(path)));
        }
    }
    let names: Vec<String> = jobs.iter().map(|(label, _)| label.clone()).collect();
    let by_name: std::collections::HashMap<&str, &Job> = jobs
        .iter()
        .map(|(label, job)| (label.as_str(), job))
        .collect();

    let failed = run_parallel(
        "Fetching...",
        &names,
        interactive,
        |name| match by_name[name] {
            Job::Artefact(entry, commit) => match artefacts.fetch(entry, commit) {
                Ok(()) => RepoStatus::Ok(format!("{} (artefact {})", name, entry.revision)),
                Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
            },
            Job::Store(path) => match sources.stores.as_ref().map(|s| s.update_path(path)) {
                Some(Ok(())) => RepoStatus::Ok(name.to_string()),
                Some(Err(e)) => RepoStatus::Fail(format!("{}: {:#}", name, e)),
                None => RepoStatus::Skip(name.to_string()),
            },
        },
        out,
        err,
    )?;
    // The reasons in the message itself: the CLI prints only the outermost
    // error of a chain, and neither failure may hide the other.
    match (failed, resolved) {
        (0, Ok(_)) => Ok(()),
        (n, Ok(_)) => anyhow::bail!(
            "{} failed to fetch",
            crate::cache::plural(n, "repo", "repos")
        ),
        (0, Err(e)) => Err(anyhow::anyhow!(
            "cannot resolve the workspace's dependencies: {:#}",
            e
        )),
        (n, Err(e)) => Err(anyhow::anyhow!(
            "{} failed to fetch; and cannot resolve the workspace's dependencies: {:#}",
            crate::cache::plural(n, "repo", "repos"),
            e
        )),
    }
}

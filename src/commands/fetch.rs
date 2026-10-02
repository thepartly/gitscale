use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::artefact::Artefacts;
use crate::config::{filter_entries, load_workspace};
use crate::progress::{run_entries, RepoStatus};
use crate::store::Sources;

pub fn run(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    if config.repos.is_empty() {
        writeln!(out, "Nothing to fetch.")?;
        return Ok(());
    }

    let sources = Sources::new(&config_root, no_cache)?;
    let artefacts = Artefacts::new(&config, &config_root, sources.images());
    // Resolving online is the fetch: every store resolution reads is brought
    // up to date on the way, so the next offline `status` sees what the
    // remotes have now — implicit dependencies included. A fetch changes no
    // checkout, so a graph that will not resolve — a remote that cannot be
    // reached, a conflict — still gets the declared entries fetched, and the
    // command fails afterwards with the reason.
    let resolved = crate::resolve::workspace(
        &config,
        &config_root,
        true,
        &sources,
        Some(&artefacts),
        verbose,
    );
    let (selected, unresolved) = match resolved {
        Ok(resolution) => (resolution.select(names)?, None),
        Err(e) => (filter_entries(&config.repos, names)?, Some(e)),
    };

    let fetched = run_entries(
        "Fetching...",
        "fetch",
        &selected,
        interactive,
        |entry| {
            let name = entry.directory.as_str();
            if config_root.join(&entry.directory).is_symlink() {
                return RepoStatus::Skip(format!("{} (symlink)", name));
            }
            if entry.is_artefact() {
                return match artefacts.fetch(entry) {
                    Ok(commit) => RepoStatus::Ok(format!(
                        "{} (artefact {})",
                        name,
                        crate::git::short_sha(&commit)
                    )),
                    Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
                };
            }
            match &sources.stores {
                // Already fetched by resolution; once per command.
                Some(stores) => match stores.update(&crate::git::remote_url(entry)) {
                    Ok(_) => RepoStatus::Ok(name.to_string()),
                    Err(e) => RepoStatus::Fail(format!("{}: {:#}", name, e)),
                },
                // CI keeps no history to fetch into: the next pull takes the
                // commit it needs.
                None => RepoStatus::Skip(format!("{} (no history in CI)", name)),
            }
        },
        out,
        err,
    );
    // The reasons in the message itself: the CLI prints only the outermost
    // error of a chain, and neither failure may hide the other.
    match (fetched, unresolved) {
        (fetched, None) => fetched,
        (Ok(()), Some(e)) => Err(anyhow::anyhow!(
            "cannot resolve the workspace's dependencies: {:#}",
            e
        )),
        (Err(failed), Some(e)) => Err(anyhow::anyhow!(
            "{:#}; and cannot resolve the workspace's dependencies: {:#}",
            failed,
            e
        )),
    }
}

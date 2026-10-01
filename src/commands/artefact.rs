//! `gitscale artefact` — the commands about artefacts themselves rather than
//! about checkouts: publishing one, and looking at what the registry holds.

use anyhow::{bail, Result};
use std::io::Write;
use std::path::Path;

use crate::artefact::Artefacts;
use crate::cache::{human_size, plural};
use crate::config::{filter_entries, load_workspace, GitScaleConfig, RepoEntry};

pub use crate::artefact::publish;

/// The artefact entries `names` selects: all of them when none are given. A
/// name that is declared but is not an artefact entry is an error, as an
/// undeclared one is.
fn selected(config: &GitScaleConfig, names: &[String]) -> Result<Vec<RepoEntry>> {
    let chosen = filter_entries(&config.repos, names)?;
    let not_artefacts: Vec<&str> = chosen
        .iter()
        .filter(|e| !e.is_artefact() && !names.is_empty())
        .map(|e| e.directory.as_str())
        .collect();
    if !not_artefacts.is_empty() {
        bail!("Not artefact entries: {}", not_artefacts.join(", "));
    }
    Ok(chosen.into_iter().filter(|e| e.is_artefact()).collect())
}

/// `gitscale artefact show` — for each artefact entry, what the remote and
/// the registry say right now and what is installed, changing nothing. The
/// one place to answer "why does it say no artefact, missing or changed".
pub fn show(root: Option<&Path>, names: &[String], out: &mut dyn Write) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    let entries = selected(&config, names)?;
    if entries.is_empty() {
        writeln!(out, "No artefact entries declared.")?;
        return Ok(());
    }
    let artefacts = Artefacts::new(&config, &config_root, None);
    let mut failed = 0;
    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            writeln!(out)?;
        }
        writeln!(out, "{}", entry.directory)?;
        let revision = if entry.revision.is_empty() {
            "(default branch)"
        } else {
            entry.revision.as_str()
        };
        let line = |out: &mut dyn Write, label: &str, value: &str| -> std::io::Result<()> {
            writeln!(out, "  {:<11}{}", label, value)
        };
        line(out, "revision", revision)?;
        let described = match artefacts.describe(entry) {
            Ok(described) => described,
            Err(e) => {
                failed += 1;
                // The first line only: a hint is printed as its own line.
                let text = format!("{:#}", e);
                let mut lines = text.lines();
                line(out, "error", lines.next().unwrap_or_default())?;
                for rest in lines {
                    line(out, "", rest)?;
                }
                continue;
            }
        };
        line(out, "image", &described.image.reference())?;
        line(out, "commit", &described.commit)?;
        match &described.digest {
            Some(digest) => line(out, "published", digest)?,
            None => line(
                out,
                "published",
                "no — its pipeline may not have published this commit yet",
            )?,
        }
        for (n, layer) in described.layers.iter().enumerate() {
            let title = if layer.title.is_empty() {
                format!("#{}", n + 1)
            } else {
                layer.title.clone()
            };
            line(
                out,
                if n == 0 { "layers" } else { "" },
                &format!("{}  {}  {}", title, human_size(layer.size), layer.digest),
            )?;
        }
        match &described.installed {
            Some(installed) => line(
                out,
                "installed",
                &format!(
                    "{} ({})",
                    installed.commit,
                    installed.digest.as_deref().unwrap_or("no digest")
                ),
            )?,
            None => line(out, "installed", "nothing")?,
        }
        let status = match (&described.installed, described.flags.is_empty()) {
            (None, _) => "not installed".to_string(),
            (Some(_), true) => "ok".to_string(),
            (Some(_), false) => described.flags.join(", "),
        };
        line(out, "status", &status)?;
    }
    if failed > 0 {
        bail!(
            "{} could not be looked up",
            plural(failed, "artefact entry", "artefact entries")
        );
    }
    Ok(())
}

/// `gitscale artefact list` — the commits each artefact entry has images
/// for in the registry, labelled with the branches and tags that point at
/// them now, and which one is installed.
pub fn list(root: Option<&Path>, names: &[String], out: &mut dyn Write) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    let entries = selected(&config, names)?;
    if entries.is_empty() {
        writeln!(out, "No artefact entries declared.")?;
        return Ok(());
    }
    let artefacts = Artefacts::new(&config, &config_root, None);
    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            writeln!(out)?;
        }
        let (image, tags) = artefacts.published(entry)?;
        let refs = crate::git::ls_remote_refs(&crate::git::remote_url(entry))?;
        let installed = artefacts.installed(entry).map(|m| m.commit);
        writeln!(
            out,
            "{}  {}  ({})",
            entry.directory,
            image.reference(),
            plural(tags.len(), "image", "images")
        )?;
        // Commits a branch or tag names now come first, in the remote's own
        // order; then the rest, which only the registry still remembers.
        let labels = |tag: &str| -> Vec<&str> {
            refs.iter()
                .filter(|(sha, _)| sha.eq_ignore_ascii_case(tag))
                .map(|(_, name)| name.as_str())
                .collect()
        };
        let mut rows: Vec<(String, Vec<&str>)> =
            tags.iter().map(|tag| (tag.clone(), labels(tag))).collect();
        rows.sort_by(|a, b| a.1.is_empty().cmp(&b.1.is_empty()).then(a.0.cmp(&b.0)));
        for (tag, names) in rows {
            let mut notes = names.join(", ");
            if installed.as_deref() == Some(tag.as_str()) {
                if !notes.is_empty() {
                    notes.push_str("  ");
                }
                notes.push_str("(installed)");
            }
            if notes.is_empty() {
                writeln!(out, "  {}", tag)?;
            } else {
                writeln!(out, "  {}  {}", tag, notes)?;
            }
        }
    }
    Ok(())
}

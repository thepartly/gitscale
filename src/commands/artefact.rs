//! `gitscale artefact` — the commands about artefacts themselves rather than
//! about checkouts: publishing one, and looking at what the registry holds.

use anyhow::{bail, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::artefact::Artefacts;
use crate::cache::{human_size, plural};
use crate::config::{load_workspace, RepoEntry};
use crate::prefer::Form;
use crate::resolve::Network;
use crate::store::Sources;

pub use crate::artefact::publish;

/// The checkouts `dirs` names among those taken as an artefact — every one
/// when none are given — with the workspace's root and artefacts. A name
/// that is no such checkout is an error.
fn selected(root: Option<&Path>, dirs: &[String]) -> Result<(PathBuf, Artefacts, Vec<RepoEntry>)> {
    let (config, config_root) = load_workspace(root)?;
    let here = crate::paths::Here::new(root, &config_root)?;
    let sources = Sources::new(&config_root, false)?;
    let artefacts = Artefacts::new(&config, &config_root, None);
    let resolution = crate::resolve::workspace_with(
        &config,
        &config_root,
        Network::OnMiss.in_ci(),
        &sources,
        Some(&artefacts),
        false,
    )?;
    let mut entries = Vec::new();
    if dirs.is_empty() {
        entries.extend(
            resolution
                .slots
                .iter()
                .filter(|s| s.form() == Form::Artefact)
                .map(|s| s.entry()),
        );
    }
    for dir in dirs {
        let slot = here.slot(&resolution, dir)?;
        if slot.form() != Form::Artefact {
            bail!("{} is not taken as an artefact", slot.directory);
        }
        entries.push(slot.entry());
    }
    Ok((config_root, artefacts, entries))
}

/// `gitscale artefact show` — for each checkout taken as an artefact, what
/// the registry says right now about the release it is at, and what is
/// installed, changing nothing. The one place to answer "why does it say no
/// artefact, missing or changed".
pub fn show(root: Option<&Path>, dirs: &[String], out: &mut dyn Write) -> Result<()> {
    let (_, artefacts, entries) = selected(root, dirs)?;
    if entries.is_empty() {
        writeln!(out, "No checkout is taken as an artefact.")?;
        return Ok(());
    }
    let mut failed = 0;
    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            writeln!(out)?;
        }
        writeln!(out, "{}", entry.directory)?;
        let line = |out: &mut dyn Write, label: &str, value: &str| -> std::io::Result<()> {
            writeln!(out, "  {:<11}{}", label, value)
        };
        line(out, "release", &entry.revision)?;
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
        match &described.digest {
            Some(digest) => line(out, "published", digest)?,
            None => line(out, "published", "no")?,
        }
        if let Some(hash) = &described.hash {
            line(out, "sources", hash)?;
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
                    installed.tag,
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
            plural(failed, "image", "images")
        );
    }
    Ok(())
}

/// `gitscale artefact list` — the releases each checkout taken as an
/// artefact has images of, newest first, each with the source hash its
/// image was built from, and which one is installed.
pub fn list(root: Option<&Path>, dirs: &[String], out: &mut dyn Write) -> Result<()> {
    let (config_root, artefacts, entries) = selected(root, dirs)?;
    if entries.is_empty() {
        writeln!(out, "No checkout is taken as an artefact.")?;
        return Ok(());
    }
    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            writeln!(out)?;
        }
        let (image, releases) = artefacts.releases(entry)?;
        let installed = crate::artefact::installed(&config_root, &entry.directory).map(|m| m.tag);
        writeln!(
            out,
            "{}  {}  ({})",
            entry.directory,
            image.reference(),
            plural(releases.len(), "release", "releases")
        )?;
        let width = releases.iter().map(|(tag, _)| tag.len()).max().unwrap_or(0);
        for (tag, hash) in releases {
            let mark = if installed.as_deref() == Some(tag.as_str()) {
                "  (installed)"
            } else {
                ""
            };
            writeln!(
                out,
                "  {:<width$}  {}{}",
                tag,
                hash.as_deref().unwrap_or("-"),
                mark
            )?;
        }
    }
    Ok(())
}

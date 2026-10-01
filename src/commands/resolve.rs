//! `gitscale resolve`: where resolution moved the root's own revisions, and
//! with `--write`, recording them in `.gitscale.toml` so the diff shows what
//! the workspace is really built from.

use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

use crate::artefact::Artefacts;
use crate::config::{load_workspace, CONFIG_FILENAME};

/// One root entry whose revision resolution changed.
struct Change {
    directory: String,
    from: String,
    to: String,
    by: String,
}

pub fn run(
    root: Option<&Path>,
    write: bool,
    verbose: bool,
    no_cache: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    let cache = crate::commands::cache::open(&config, no_cache);
    let artefacts = Artefacts::new(&config, &config_root, cache.clone());
    let resolution = crate::resolve::workspace(
        &config,
        &config_root,
        true,
        cache,
        Some(&artefacts),
        verbose,
    )?;

    // Only entries that already give a revision: one without leaves it to
    // the dependencies on purpose, and an override is the root's own choice.
    let mut changes = Vec::new();
    for entry in &config.repos {
        if entry.revision.is_empty() || entry.is_override {
            continue;
        }
        let Some(slot) = resolution.slot(&entry.directory) else {
            continue;
        };
        let Some(chosen) = &slot.chosen else {
            continue;
        };
        if chosen.revision == entry.revision {
            continue;
        }
        changes.push(Change {
            directory: entry.directory.clone(),
            from: entry.revision.clone(),
            to: chosen.revision.clone(),
            by: chosen.by.clone(),
        });
    }

    if changes.is_empty() {
        writeln!(
            out,
            "Every root entry already declares the revision it resolves to."
        )?;
        return Ok(());
    }
    let width = changes
        .iter()
        .map(|c| c.directory.chars().count())
        .max()
        .unwrap_or(0);
    let from_width = changes
        .iter()
        .map(|c| c.from.chars().count())
        .max()
        .unwrap_or(0);
    let to_width = changes
        .iter()
        .map(|c| c.to.chars().count())
        .max()
        .unwrap_or(0);
    for change in &changes {
        let line = format!(
            "{:<width$}   {:<from_width$} → {:<to_width$}   raised by {}",
            change.directory,
            change.from,
            change.to,
            change.by,
            width = width,
            from_width = from_width,
            to_width = to_width,
        );
        writeln!(out, "{}", line)?;
    }

    let count = crate::cache::plural(changes.len(), "entry", "entries");
    if !write {
        writeln!(
            out,
            "{} would change; run with --write to update {}",
            count, CONFIG_FILENAME
        )?;
        return Ok(());
    }
    write_revisions(&config_root.join(CONFIG_FILENAME), &changes)?;
    writeln!(out, "Updated {} in {}", count, CONFIG_FILENAME)?;
    Ok(())
}

/// Set each changed entry's `revision` in place, leaving every comment, key
/// order and table gitscale does not know about as it was.
fn write_revisions(path: &Path, changes: &[Change]) -> Result<()> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("{}: invalid TOML", path.display()))?;
    for change in changes {
        let entry = doc
            .get_mut("repos")
            .and_then(|repos| repos.get_mut(&change.directory))
            .with_context(|| format!("{}: no entry repos.{}", path.display(), change.directory))?;
        let value = if let Some(table) = entry.as_inline_table_mut() {
            table.get_mut("revision")
        } else if let Some(table) = entry.as_table_mut() {
            table
                .get_mut("revision")
                .and_then(|item| item.as_value_mut())
        } else {
            None
        }
        .with_context(|| {
            format!(
                "{}: repos.{} has no revision to update",
                path.display(),
                change.directory
            )
        })?;
        let decor = value.decor().clone();
        *value = toml_edit::Value::from(change.to.as_str());
        *value.decor_mut() = decor;
    }
    std::fs::write(path, doc.to_string())
        .with_context(|| format!("cannot write {}", path.display()))
}

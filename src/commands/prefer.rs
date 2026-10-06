//! `git scale prefer`: record how checkouts arrive — their sources, their
//! published artefact, or the sources with the artefact laid over them — or
//! show what is recorded. It only records: the next placement applies it.
//! See [`crate::prefer`].

use anyhow::{bail, Result};
use std::io::Write;
use std::path::Path;

use crate::config::load_workspace;
use crate::prefer::{Form, Prefs};
use crate::store::Sources;

pub fn run(
    root: Option<&Path>,
    form: Option<Form>,
    dirs: &[String],
    no_cache: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    let here = crate::paths::Here::new(root, &config_root)?;
    let sources = Sources::new(&config_root, no_cache)?;
    let resolution =
        crate::resolve::workspace(&config, &config_root, false, &sources, None, false)?;
    let mut prefs = Prefs::load(&config_root)?;
    let slots = dirs
        .iter()
        .map(|dir| here.slot(&resolution, dir))
        .collect::<Result<Vec<_>>>()?;

    let Some(form) = form else {
        let shown: Vec<(String, Form)> = if slots.is_empty() {
            prefs
                .iter()
                .map(|(url, form)| {
                    let checkouts: Vec<&str> = resolution
                        .slots_of(url)
                        .map(|s| s.directory.as_str())
                        .collect();
                    let name = if checkouts.is_empty() {
                        format!("{} (not checked out)", url)
                    } else {
                        checkouts.join(", ")
                    };
                    (name, form)
                })
                .collect()
        } else {
            slots
                .iter()
                .map(|s| {
                    (
                        s.directory.clone(),
                        prefs.get(&s.url).unwrap_or(Form::Source),
                    )
                })
                .collect()
        };
        if shown.is_empty() {
            writeln!(out, "Every checkout takes its sources.")?;
        }
        let width = shown
            .iter()
            .map(|(n, _)| n.chars().count())
            .max()
            .unwrap_or(0);
        for (name, form) in shown {
            writeln!(out, "{:<width$}   {}", name, form, width = width)?;
        }
        return Ok(());
    };

    if slots.is_empty() {
        bail!(
            "--{} names the checkouts it is for: git scale prefer --{} <DIR>...",
            form,
            form
        );
    }
    for slot in &slots {
        prefs.set(&slot.url, form);
    }
    prefs.save(&config_root)?;
    for slot in &slots {
        writeln!(out, "{}   {}", slot.directory, form)?;
    }
    writeln!(out, "The next placement applies it: git scale pull")?;
    Ok(())
}

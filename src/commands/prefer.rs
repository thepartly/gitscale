//! `git scale prefer`: record how checkouts arrive — their sources, their
//! published artefact, or the sources with the artefact laid over them — and
//! whether a lazy one is there at all, or show what is recorded. It only
//! records: the next placement applies it, except that a checkout made lazy
//! goes at once when that loses nothing. See [`crate::prefer`].

use anyhow::{bail, Result};
use std::io::Write;
use std::path::Path;

use crate::config::load_workspace;
use crate::prefer::{Form, Prefs, Presence};
use crate::store::Sources;

/// What to record for the checkouts named.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Change {
    pub form: Option<Form>,
    /// `Some(None)`: back to what the configs say.
    pub presence: Option<Option<Presence>>,
}

impl Change {
    /// `--unset`: no preference of either kind, as `git config --unset`
    /// leaves no value. Sources are the default form, so recording them
    /// removes the form preference.
    pub const UNSET: Change = Change {
        form: Some(Form::Source),
        presence: Some(None),
    };
}

/// A checkout `prefer` was named: placed, or lazy and left out.
struct Named {
    directory: String,
    url: String,
    /// Every request for it says `lazy`.
    committed_lazy: bool,
    on_topic: bool,
    /// A repository whose config asks for it without `lazy`, keeping it
    /// placed.
    required_by: Option<String>,
}

pub fn run(
    root: Option<&Path>,
    change: Change,
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
    let named = dirs
        .iter()
        .map(|dir| {
            if let Some(entry) = crate::resolve::asleep(&here, &resolution, dir) {
                return Ok(Named {
                    directory: entry.directory.clone(),
                    url: entry.repo_url.clone(),
                    committed_lazy: entry.lazy,
                    on_topic: false,
                    required_by: None,
                });
            }
            let slot = here.slot(&resolution, dir)?;
            Ok(Named {
                directory: slot.directory.clone(),
                url: slot.url.clone(),
                committed_lazy: slot.lazy,
                on_topic: slot.topic.is_some(),
                required_by: slot.required_by.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    if change == Change::default() {
        return show(&resolution, &prefs, &named, out);
    }
    if named.is_empty() {
        let flag = match (change.form, change.presence) {
            _ if change == Change::UNSET => "unset".to_string(),
            (Some(form), _) => form.to_string(),
            (None, Some(Some(presence))) => presence.to_string(),
            _ => "unset".to_string(),
        };
        bail!(
            "--{} names the checkouts it is for: git scale prefer --{} <DIR>...",
            flag,
            flag
        );
    }
    for n in &named {
        if let Some(form) = change.form {
            prefs.set(&n.url, form);
        }
        if let Some(presence) = change.presence {
            prefs.set_presence(&n.url, presence);
        }
    }
    prefs.save(&config_root)?;
    for n in &named {
        let lazy = match prefs.presence(&n.url) {
            Some(p) => p == Presence::Lazy,
            None => n.committed_lazy,
        };
        writeln!(
            out,
            "{}   {}",
            n.directory,
            describe(&prefs, &n.url, n.committed_lazy, n.required_by.as_deref())
        )?;
        if !lazy || n.on_topic {
            continue;
        }
        if let Some(by) = &n.required_by {
            writeln!(
                out,
                "  kept  {} ({} requires it without lazy = true)",
                n.directory, by
            )?;
            continue;
        }
        // Made lazy: nothing keeps it, unless it holds work. Its commits
        // stay in its store.
        match crate::checkout::drop_lazy(
            &config_root,
            &n.directory,
            &n.url,
            &resolution.planted_in(&n.directory),
            sources.stores.as_ref(),
        )? {
            crate::checkout::Dropped::Removed => writeln!(out, "  remove  {} (lazy)", n.directory)?,
            crate::checkout::Dropped::Kept(work) => writeln!(
                out,
                "  kept  {} ({}: commit and push it, or discard it, then git scale prefer \
                 --lazy {} again)",
                n.directory, work, n.directory
            )?,
            crate::checkout::Dropped::Absent => {}
        }
    }
    writeln!(out, "The next placement applies it: git scale pull")?;
    Ok(())
}

/// A checkout's form, and its presence when that is not simply there.
fn describe(prefs: &Prefs, url: &str, committed_lazy: bool, required_by: Option<&str>) -> String {
    let form = prefs.get(url).unwrap_or(Form::Source);
    match (prefs.presence(url), committed_lazy) {
        (Some(Presence::Lazy), _) if required_by.is_some() => format!(
            "{}, lazy, but placed: {} requires it",
            form,
            required_by.unwrap_or_default()
        ),
        (Some(p), _) => format!("{}, {}", form, p),
        (None, true) => format!("{}, lazy (committed)", form),
        (None, false) => form.to_string(),
    }
}

/// The preferences of the checkouts named, or every preference with the
/// checkouts it applies to.
fn show(
    resolution: &crate::resolution::Resolution,
    prefs: &Prefs,
    named: &[Named],
    out: &mut dyn Write,
) -> Result<()> {
    let shown: Vec<(String, String)> = if named.is_empty() {
        let mut urls: Vec<&str> = prefs
            .iter()
            .map(|(url, _)| url)
            .chain(prefs.presences().map(|(url, _)| url))
            .collect();
        urls.sort();
        urls.dedup();
        urls.into_iter()
            .map(|url| {
                let checkouts: Vec<&str> = resolution
                    .slots_of(url)
                    .map(|s| s.directory.as_str())
                    .chain(
                        resolution
                            .asleep
                            .iter()
                            .filter(|e| crate::urls::normalize(&e.repo_url) == url)
                            .map(|e| e.directory.as_str()),
                    )
                    .collect();
                let name = if checkouts.is_empty() {
                    format!("{} (not checked out)", url)
                } else {
                    checkouts.join(", ")
                };
                let committed_lazy = resolution.slots_of(url).any(|s| s.lazy)
                    || resolution
                        .asleep
                        .iter()
                        .any(|e| crate::urls::normalize(&e.repo_url) == url && e.lazy);
                let required_by = resolution
                    .slots_of(url)
                    .find_map(|s| s.required_by.as_deref());
                (name, describe(prefs, url, committed_lazy, required_by))
            })
            .collect()
    } else {
        named
            .iter()
            .map(|n| {
                (
                    n.directory.clone(),
                    describe(prefs, &n.url, n.committed_lazy, n.required_by.as_deref()),
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
    Ok(())
}

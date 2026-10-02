//! `gitscale cache` — the commands that own the CI cache directly.
//!
//! In CI, `pull` and `fetch` use the cache by themselves and say nothing about
//! it. These work on it anywhere, `CI` set or not: to see what it holds, to
//! warm it — a runner image built ahead of time — and to keep it from growing
//! without bound.

use anyhow::Result;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use crate::cache::{self, Cache};
use crate::config::{find_config, load_config, load_workspace, GitScaleConfig, RepoEntry};
use crate::progress::{run_parallel, RepoStatus};

/// The cache at its usual location, or an error saying there is none.
fn open() -> Result<Cache> {
    Cache::open().ok_or_else(|| {
        anyhow::anyhow!("no cache location: set GITSCALE_CACHE_DIR, XDG_DATA_HOME or HOME")
    })
}

/// `gitscale cache update` — bring entries up to date without touching any
/// checkout: a snapshot pin for each git entry's revision, and the image each
/// artefact entry's revision names — exactly what a CI job would take.
pub fn update(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    let cache = open()?;
    if config.repos.is_empty() {
        writeln!(out, "Nothing to cache.")?;
        return Ok(());
    }
    // The revisions resolution settles on, implicit dependencies included:
    // what the next job will ask the cache for.
    let sources = crate::store::Sources {
        stores: None,
        cache: Some(cache.clone()),
    };
    let artefacts = crate::artefact::Artefacts::new(&config, &config_root, sources.images());
    let resolution = crate::resolve::workspace(
        &config,
        &config_root,
        true,
        &sources,
        Some(&artefacts),
        verbose,
    )?;
    let selected = resolution.select(names)?;
    let names: Vec<String> = selected.iter().map(|e| e.directory.clone()).collect();
    let by_name: std::collections::HashMap<&str, &RepoEntry> =
        selected.iter().map(|e| (e.directory.as_str(), e)).collect();

    let failed = run_parallel(
        "Updating cache entries...",
        &names,
        interactive,
        |name| {
            let entry = &by_name[name];
            if entry.is_artefact() {
                return match artefacts.warm(entry) {
                    Ok(Some(commit)) => RepoStatus::Ok(format!(
                        "{} (artefact {})",
                        name,
                        crate::git::short_sha(&commit)
                    )),
                    Ok(None) => RepoStatus::Skip(format!("{} (no image store)", name)),
                    Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
                };
            }
            let commit = resolution
                .slot(name)
                .and_then(|s| s.commit.clone())
                .unwrap_or_else(|| entry.revision.clone());
            match cache.pin(&crate::git::remote_url(entry), &commit) {
                Ok(Some(pinned)) => RepoStatus::Ok(format!(
                    "{} (pinned at {})",
                    name,
                    crate::git::short_sha(&pinned.sha)
                )),
                Ok(None) => RepoStatus::Skip(format!("{} (nothing to pin)", name)),
                Err(e) => RepoStatus::Fail(format!("{}: {}", name, e)),
            }
        },
        out,
        err,
    )?;

    if verbose {
        let usage = cache.usage();
        writeln!(
            out,
            "Cache at {}: {}, {}",
            cache.root().display(),
            cache::plural(usage.entries, "entry", "entries"),
            cache::human_size(usage.bytes)
        )?;
    }
    if failed > 0 {
        anyhow::bail!(
            "{} failed to update",
            cache::plural(failed, "cache entry", "cache entries")
        );
    }
    Ok(())
}

/// The config `status` uses when there is one, to name the entries this
/// workspace declares. Not needed — the cache belongs to the user, not to a
/// workspace — but one that does not parse is an error, not a reason to
/// guess.
fn optional_workspace_config(root: Option<&Path>) -> Result<GitScaleConfig> {
    match find_config(root) {
        Ok(path) => load_config(&path),
        Err(_) => Ok(GitScaleConfig::default()),
    }
}

/// `gitscale cache status` — what the cache holds, entry by entry: which
/// repository each belongs to, what it is holding, and how long since
/// anything wanted it — which is what decides whether `compact` would take
/// it.
pub fn status(root: Option<&Path>, out: &mut dyn Write) -> Result<()> {
    let config = optional_workspace_config(root)?;
    let cache = open()?;

    // Entry directory name -> the directory this workspace declares it under.
    let declared: std::collections::HashMap<String, &str> = config
        .repos
        .iter()
        .map(|e| {
            let url = crate::git::remote_url(e);
            let name = if e.is_artefact() {
                crate::store::image_entry_name(&url)
            } else {
                crate::store::entry_name(&url)
            };
            (name, e.directory.as_str())
        })
        .collect();

    // One row per repository, not per entry: a repository consumed both as
    // source and as an artefact has an entry of each kind.
    let mut rows: BTreeMap<String, Row> = BTreeMap::new();
    for entry in cache.stats() {
        let name = entry
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // An entry this workspace does not declare still belongs to somebody:
        // name it as the cache does rather than hide it.
        let repo = declared.get(&name).map(|d| d.to_string()).unwrap_or(name);
        let row = rows.entry(repo).or_default();
        match entry.kind {
            cache::Kind::Snapshot => row.snapshot += entry.bytes,
            cache::Kind::Image => row.image += entry.bytes,
        }
        row.last_used = row.last_used.max(entry.last_used);
        row.revisions.extend(entry.revisions);
    }

    let usage = cache.usage();
    writeln!(
        out,
        "cache  {}  ({}, {})",
        cache.root().display(),
        cache::plural(usage.entries, "entry", "entries"),
        cache::human_size(usage.bytes)
    )?;
    if rows.is_empty() {
        writeln!(out, "\nNothing cached yet.")?;
        return Ok(());
    }

    let headers = ["REPO", "SNAPSHOTS", "IMAGES", "TOTAL", "REVS", "LAST USED"];
    let cells: Vec<[String; 6]> = rows
        .iter()
        .map(|(repo, row)| {
            [
                repo.clone(),
                size_or_dash(row.snapshot),
                size_or_dash(row.image),
                size_or_dash(row.snapshot + row.image),
                row.revisions.len().to_string(),
                cache::ago(row.last_used),
            ]
        })
        .collect();

    let mut widths = [0usize; 6];
    for (i, header) in headers.iter().enumerate() {
        widths[i] = header.len().max(
            cells
                .iter()
                .map(|r| r[i].chars().count())
                .max()
                .unwrap_or(0),
        );
    }
    let last = headers.len() - 1;
    // Sizes and counts read better against the right edge of their columns.
    let line = |cells: &[String; 6]| {
        cells
            .iter()
            .enumerate()
            .map(|(i, cell)| match i {
                i if i == last => cell.clone(),
                1..=4 => format!("{:>width$}", cell, width = widths[i]),
                _ => format!("{:<width$}", cell, width = widths[i]),
            })
            .collect::<Vec<_>>()
            .join("  ")
    };

    writeln!(out)?;
    writeln!(out, "  {}", line(&headers.map(|h| h.to_string())))?;
    for (row, rendered) in rows.values().zip(&cells) {
        writeln!(out, "  {}", line(rendered))?;
        // Every pin and image listed with its own age: they are what
        // `compact` drops one at a time.
        for revision in &row.revisions {
            writeln!(
                out,
                "      {}  {}",
                short_revision(&revision.name),
                cache::ago(revision.last_used)
            )?;
        }
    }
    Ok(())
}

/// One repository's line: what each kind of entry costs it, and what they
/// hold.
#[derive(Default)]
struct Row {
    snapshot: u64,
    image: u64,
    last_used: Option<std::time::SystemTime>,
    revisions: Vec<cache::Revision>,
}

fn size_or_dash(bytes: u64) -> String {
    if bytes == 0 {
        "-".to_string()
    } else {
        cache::human_size(bytes)
    }
}

/// A pin ref or an image's commit as something to read: `pin/<40 hex>` is
/// the ref git needs, not a name anyone scans a column for.
fn short_revision(name: &str) -> String {
    match name.strip_prefix("pin/") {
        Some(sha) => crate::git::short_sha(sha).to_string(),
        None if crate::git::is_full_sha(name) => crate::git::short_sha(name).to_string(),
        None => name.to_string(),
    }
}

/// `gitscale cache compact` — evict what nothing has used for `keep_recent`:
/// whole entries, then the pins and images inside the ones that stay.
pub fn compact(keep_recent: &str, out: &mut dyn Write) -> Result<()> {
    let keep = cache::parse_period(keep_recent)?;
    let cache = open()?;
    let before = cache.usage();
    let report = cache.compact(keep)?;
    let after = cache.usage();
    writeln!(
        out,
        "Compacted {}: {} evicted, {} dropped, {} freed.",
        cache.root().display(),
        cache::plural(report.entries, "entry", "entries"),
        cache::plural(report.pins, "pin", "pins"),
        cache::human_size(before.bytes.saturating_sub(after.bytes))
    )?;
    writeln!(
        out,
        "{} left, {}.",
        cache::plural(after.entries, "entry", "entries"),
        cache::human_size(after.bytes)
    )?;
    Ok(())
}

//! `gitscale cache` — the commands that own the cache directly.
//!
//! Everything else touches it implicitly: a clone, fetch or pull updates the
//! entries it is about to read and says nothing about it. These three are for
//! the times that is not enough — warming a repo nobody has pulled yet,
//! rebuilding an entry something deleted, and keeping the directory from
//! growing without bound.

use anyhow::Result;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use crate::cache::{self, Cache};
use crate::commands::clone::filter_entries;
use crate::config::{find_config, load_config, GitScaleConfig, RepoEntry};
use crate::git::is_ci;
use crate::progress::{run_parallel, RepoStatus};

/// The cache a command should use: what the config asks for, unless
/// `--no-cache` was given.
pub fn open(config: &GitScaleConfig, no_cache: bool) -> Option<Cache> {
    if no_cache {
        return None;
    }
    Cache::open(&config.cache)
}

/// Relink the workspace's own root repository to the cache, if the config asks
/// for it.
///
/// Opt-in, and it stays opt-in: adopting deletes the root's own objects and
/// converts a repository that stood on its own into one that depends on the
/// cache. A root `gitscale clone` created is already linked, so this is only
/// ever about one somebody cloned with plain `git clone`.
pub fn adopt_root(
    cache: Option<&Cache>,
    config: &GitScaleConfig,
    config_root: &Path,
    out: &mut dyn Write,
) -> Result<()> {
    let Some(cache) = cache else {
        return Ok(());
    };
    if !config.cache.adopt_root || !crate::git::is_repo_root(config_root) {
        return Ok(());
    }
    let Some(url) = crate::git::origin_url(config_root) else {
        return Ok(());
    };
    if cache.adopt(config_root, &url)? {
        writeln!(out, "  adopt {} into the cache", config_root.display())?;
    }
    Ok(())
}

/// `gitscale cache update` — bring entries up to date without touching any
/// checkout. The one command that warms a repo nobody has pulled yet.
pub fn update(
    root: Option<&Path>,
    names: &[String],
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (config, _config_root) = load(root)?;
    let Some(cache) = open(&config, no_cache) else {
        writeln!(out, "The object cache is off.")?;
        return Ok(());
    };
    let selected = cacheable(&config, names)?;
    if selected.is_empty() {
        writeln!(out, "Nothing to cache.")?;
        return Ok(());
    }

    let ci = is_ci();
    let names: Vec<String> = selected.iter().map(|e| e.directory.clone()).collect();
    let by_name: std::collections::HashMap<&str, &RepoEntry> =
        selected.iter().map(|e| (e.directory.as_str(), e)).collect();

    let failed = run_parallel(
        "Updating cache entries...",
        &names,
        interactive,
        |name| {
            let entry = &by_name[name];
            let url = crate::git::remote_url(entry);
            let result = if ci {
                cache.pin(&url, &entry.revision).map(|pinned| match pinned {
                    Some(pinned) => Some(format!("{} (pinned at {})", name, short(&pinned.sha))),
                    None => None,
                })
            } else {
                cache.mirror(&url).map(|_| Some(name.to_string()))
            };
            match result {
                Ok(Some(message)) => RepoStatus::Ok(message),
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

/// `gitscale cache status` — what the cache holds, entry by entry.
///
/// The summary line under `gitscale status` answers "where is it and what is
/// it costing me". This answers the next question: which repository each entry
/// belongs to, what it is holding, and how long since anything wanted it —
/// which is what decides whether `compact` would take it.
pub fn status(
    root: Option<&Path>,
    verbose: bool,
    no_cache: bool,
    out: &mut dyn Write,
) -> Result<()> {
    // As with `compact`: the cache belongs to the user, so this works from
    // anywhere. A config is used when there is one, to name the entries this
    // workspace declares and to honour `[cache] dir`.
    let (config, _) = load(root).unwrap_or_default();
    let Some(cache) = open(&config, no_cache) else {
        writeln!(out, "The object cache is off.")?;
        return Ok(());
    };

    // Entry directory name -> the directory this workspace declares it under.
    let declared: std::collections::HashMap<String, &str> = config
        .repos
        .iter()
        .filter(|e| !e.is_artefact())
        .map(|e| {
            (
                cache::entry_name(&crate::git::remote_url(e)),
                e.directory.as_str(),
            )
        })
        .collect();

    // One row per repository, not per entry: a repository can have a mirror and
    // a snapshot at once — a developer machine that also runs jobs — and what
    // it costs is both of them.
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
            cache::Kind::Mirror => row.mirror += entry.bytes,
            cache::Kind::Snapshot => row.snapshot += entry.bytes,
        }
        row.last_used = row.last_used.max(entry.last_used);
        // Tagged with the kind that holds them: a snapshot's pins are worth
        // listing every time, a mirror's refs only when asked for.
        row.revisions
            .extend(entry.revisions.into_iter().map(|rev| (entry.kind, rev)));
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

    let headers = ["REPO", "MIRROR", "SNAPSHOTS", "TOTAL", "REVS", "LAST USED"];
    let cells: Vec<[String; 6]> = rows
        .iter()
        .map(|(repo, row)| {
            [
                repo.clone(),
                size_or_dash(row.mirror),
                size_or_dash(row.snapshot),
                size_or_dash(row.mirror + row.snapshot),
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
        for (kind, revision) in &row.revisions {
            // Pins are listed every time: a snapshot holds a handful, and they
            // are what `compact` drops one at a time. A mirror's refs are
            // every branch and tag the remote has, which is not a summary, so
            // those are counted above and listed only on request.
            if *kind == cache::Kind::Mirror && !verbose {
                continue;
            }
            writeln!(
                out,
                "      {}{}",
                short_revision(&revision.name),
                match cache::ago(revision.last_used) {
                    // A mirror's refs are kept as a set, not one by one, so
                    // there is no per-ref age to report.
                    age if age == "unknown" => String::new(),
                    age => format!("  {}", age),
                }
            )?;
        }
    }
    Ok(())
}

/// One repository's line: what each kind of entry costs it, and what they hold.
#[derive(Default)]
struct Row {
    mirror: u64,
    snapshot: u64,
    last_used: Option<std::time::SystemTime>,
    revisions: Vec<(cache::Kind, cache::Revision)>,
}

fn size_or_dash(bytes: u64) -> String {
    if bytes == 0 {
        "-".to_string()
    } else {
        cache::human_size(bytes)
    }
}

/// A pin ref as something to read: `pin/<40 hex>` is the ref git needs, not a
/// name anyone scans a column for.
fn short_revision(name: &str) -> String {
    match name.strip_prefix("pin/") {
        Some(sha) => sha.chars().take(12).collect(),
        None => name.to_string(),
    }
}

/// `gitscale cache repair` — re-mirror the entries this workspace borrows from
/// but that are no longer there.
///
/// A workspace whose alternate has been deleted cannot read its own history,
/// and git reports nothing until something tries to read an object. This is
/// the way back.
pub fn repair(
    root: Option<&Path>,
    names: &[String],
    no_cache: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = load(root)?;
    let Some(cache) = open(&config, no_cache) else {
        writeln!(out, "The object cache is off.")?;
        return Ok(());
    };
    let selected = cacheable(&config, names)?;

    let mut repaired = 0;
    for entry in &selected {
        let checkout = config_root.join(&entry.directory);
        if !checkout.is_dir() || checkout.is_symlink() {
            continue;
        }
        let url = crate::git::remote_url(entry);
        match cache.repair(&checkout, &url)? {
            Some(true) => {
                repaired += 1;
                writeln!(out, "  repair {}", entry.directory)?;
            }
            Some(false) => writeln!(out, "  ok     {}", entry.directory)?,
            None => writeln!(out, "  skip   {} (not borrowing)", entry.directory)?,
        }
    }
    // The root repository borrows too, once it has been adopted.
    if let Some(url) = crate::git::origin_url(&config_root) {
        if let Some(true) = cache.repair(&config_root, &url)? {
            repaired += 1;
            writeln!(out, "  repair .")?;
        }
    }
    writeln!(
        out,
        "Repaired {}.",
        cache::plural(repaired, "entry", "entries")
    )?;
    Ok(())
}

/// `gitscale cache compact` — repack every entry and evict the ones nothing
/// has used for `keep_recent`.
pub fn compact(
    root: Option<&Path>,
    keep_recent: &str,
    no_cache: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let keep = cache::parse_period(keep_recent)?;
    // A config is not required here: the cache belongs to the user, not to any
    // one workspace, so this works from anywhere. One is used when there is
    // one, so that `[cache] dir` is honoured.
    let config = find_config(root)
        .ok()
        .and_then(|path| load_config(&path).ok())
        .unwrap_or_default();
    let Some(cache) = open(&config, no_cache) else {
        writeln!(out, "The object cache is off.")?;
        return Ok(());
    };
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

fn load(root: Option<&Path>) -> Result<(GitScaleConfig, std::path::PathBuf)> {
    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    Ok((load_config(&config_path)?, config_root))
}

/// The selected entries a cache can hold anything for. An artefact is an
/// unpacked archive with no object store, so it never has an entry.
fn cacheable(config: &GitScaleConfig, names: &[String]) -> Result<Vec<RepoEntry>> {
    Ok(filter_entries(&config.repos, names)?
        .into_iter()
        .filter(|e| !e.is_artefact())
        .collect())
}

fn short(sha: &str) -> String {
    sha.chars().take(8).collect()
}

use anyhow::Result;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::cache::{self, Cache};
use crate::commands::cache as cache_cmd;
use crate::config::{find_config, load_config, load_config_optional, CONFIG_FILENAME};
use crate::git::{fetch_repo, get_artefact_status, get_repo_status, is_ci, RepoStatus};
use crate::resolve::resolve_recursive;
use crate::share;
use crate::storage::fetch_artefact;

pub fn run(
    root: Option<&Path>,
    do_fetch: bool,
    output_format: &str,
    verbose: bool,
    no_cache: bool,
    out: &mut dyn Write,
    _err: &mut dyn Write,
) -> Result<()> {
    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;

    if config.repos.is_empty() {
        writeln!(out, "No repos declared in .gitscale.toml")?;
        return Ok(());
    }

    let cache = cache_cmd::open(&config, no_cache);

    if do_fetch {
        let ci = is_ci();
        let workspace = share::source_workspace(&config_root);
        for entry in &config.repos {
            let dest = config_root.join(&entry.directory);
            if entry.is_artefact() {
                if !config.storage_url.is_empty() {
                    let revision = if entry.revision.is_empty() {
                        "HEAD"
                    } else {
                        &entry.revision
                    };
                    if verbose {
                        writeln!(out, "Fetching {}...", entry.directory)?;
                    }
                    let _ = fetch_artefact(&config.storage_url, &entry.repo_url, revision, &dest);
                }
                continue;
            }
            if dest.exists() {
                if verbose {
                    writeln!(out, "Fetching {}...", entry.directory)?;
                }
                let from = cache::source_for(
                    cache.as_ref(),
                    workspace.as_deref(),
                    entry,
                    config.share.dissociate,
                    ci,
                    verbose,
                );
                let _ = fetch_repo(entry, &config_root, &from);
            }
        }
    }

    let mut statuses: Vec<RepoStatus> = Vec::new();

    // Resolved whether or not the cache is switched on: `--no-cache` changes
    // what the next command does, not where the objects a checkout already
    // borrows happen to live.
    let cache_root = cache::resolve_dir(&config.cache.dir);

    for entry in &config.repos {
        if entry.is_artefact() {
            statuses.push(get_artefact_status(entry, &config_root));
        } else {
            let mut status = get_repo_status(entry, &config_root);
            let url = crate::git::remote_url(entry);
            if status.exists && !status.is_symlink {
                status.cache = cache::cache_use(
                    &config_root.join(&entry.directory),
                    &url,
                    cache_root.as_deref(),
                );
            }
            statuses.push(status);
        }
    }

    // Check for expected symlinks that are no longer symlinks
    let mut orphans: Vec<crate::resolve::OrphanLink> = Vec::new();
    if let Ok((symlinks, _)) = resolve_recursive(&config.repos, &config_root) {
        for sym in &symlinks {
            let link_abs = config_root.join(&sym.link_path);
            if link_abs.exists()
                && !link_abs
                    .symlink_metadata()
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
            {
                // Find the parent repo that owns this link
                let parent_dir = sym
                    .link_path
                    .iter()
                    .next()
                    .and_then(|c| c.to_str())
                    .unwrap_or("");
                if let Some(s) = statuses.iter_mut().find(|s| s.directory == parent_dir) {
                    s.has_unlinked = true;
                    if is_tree_modified(&link_abs) {
                        s.has_unlinked_modified = true;
                    }
                }
            }
        }

        // Collect orphaned symlinks (declared deps that were removed from config)
        orphans = crate::resolve::find_orphan_links(&config.repos, &config_root, &symlinks);
    }

    if output_format == "json" {
        print_json(&statuses, &orphans, cache.as_ref(), out)?;
    } else {
        print_table(&statuses, &orphans, out)?;
    }
    Ok(())
}

/// Check if a git repo at `path` (or any of its nested gitscale children) has
/// uncommitted changes or unpushed commits.
fn is_tree_modified(path: &Path) -> bool {
    // Check if this repo itself is dirty
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(path)
        .stdin(Stdio::null())
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);
    if dirty {
        return true;
    }

    // Check if there are unpushed commits
    let ahead = Command::new("git")
        .args(["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])
        .current_dir(path)
        .stdin(Stdio::null())
        .output()
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let parts: Vec<&str> = s.split_whitespace().collect();
            parts
                .first()
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(0)
                > 0
        })
        .unwrap_or(false);
    if ahead {
        return true;
    }

    // Recurse into gitscale children if this repo has a .gitscale.toml
    let child_config_path = path.join(CONFIG_FILENAME);
    if let Some(child_config) = load_config_optional(&child_config_path) {
        for entry in &child_config.repos {
            if entry.is_artefact() {
                continue;
            }
            let child_path = path.join(&entry.directory);
            if child_path.is_dir() && !child_path.is_symlink() {
                if is_tree_modified(&child_path) {
                    return true;
                }
            }
        }
    }

    false
}

fn get_status_flags(s: &RepoStatus) -> String {
    if !s.exists {
        return "missed".to_string();
    }
    if s.is_symlink {
        return "symlink".to_string();
    }
    let mut flags: Vec<String> = Vec::new();
    if s.has_unlinked && s.has_unlinked_modified {
        flags.push("unlinked".to_string());
        flags.push("modified".to_string());
    } else if s.has_unlinked {
        flags.push("unlinked".to_string());
    }
    if !s.is_clean {
        flags.push("dirty".to_string());
    }
    // The checkout borrows from an object store that is no longer there: it
    // cannot read its own history, and git says nothing until something tries
    // to read an object. `gitscale cache repair` is the way back.
    if s.cache == crate::cache::CacheUse::Broken {
        flags.push("cache-broken".to_string());
    }
    if s.is_stale {
        flags.push("stale".to_string());
    }
    if s.is_detached {
        flags.push("detached".to_string());
    }
    if s.ahead > 0 {
        flags.push(format!("+{}", s.ahead));
    }
    if s.behind > 0 {
        flags.push(format!("-{}", s.behind));
    }
    // A tag or a SHA is checked out detached, so REF reads as a commit and can
    // never equal the revision as text. What decides it is where HEAD actually
    // is: detached at the pinned commit is right, detached anywhere else is
    // the mismatch this flag is for — and used to miss.
    if !s.expected_ref.is_empty()
        && s.current_ref != s.expected_ref
        && !(s.is_detached && s.at_expected)
        && s.current_ref != "artefact"
    {
        flags.push("ref-mismatch".to_string());
    }
    if flags.is_empty() {
        "ok".to_string()
    } else {
        flags.join(", ")
    }
}

fn status_icon(flags: &str) -> &str {
    if flags == "ok" {
        return "✔";
    }
    if flags.contains("symlink") {
        return "⤷";
    }
    if flags.contains("missed") {
        return "✘";
    }
    if flags.contains("orphan") {
        return "⊘";
    }
    if flags.contains("unlinked") {
        return "~";
    }
    if flags.contains("dirty") || flags.contains("cache-broken") {
        return "!";
    }
    if flags.contains("stale") || flags.contains("ref-mismatch") {
        return "≠";
    }
    let has_ahead = flags.contains('+');
    let has_behind = flags.contains('-');
    if has_ahead && has_behind {
        return "⇅";
    }
    if has_ahead {
        return "⇑";
    }
    if has_behind {
        return "⇓";
    }
    "◆"
}

fn status_color(flags: &str) -> &str {
    if flags == "ok" {
        return "32"; // green
    }
    if flags.contains("symlink") {
        return "36"; // cyan
    }
    if flags.contains("missed") {
        return "31"; // red
    }
    if flags == "orphan" {
        return "33"; // yellow: safe to remove, target still valid
    }
    if flags.contains("orphan") {
        return "91"; // bright red: broken orphan
    }
    if flags == "unlinked" {
        return "33"; // yellow
    }
    if flags.contains("dirty")
        || flags.contains("ref-mismatch")
        || flags.contains("stale")
        || flags.contains("unlinked")
        || flags.contains("cache-broken")
    {
        return "91"; // bright red
    }
    if flags.contains('+') || flags.contains('-') {
        return "33"; // yellow
    }
    "36" // cyan
}

fn colorize(text: &str, ansi_code: &str, bold: bool) -> String {
    if bold {
        format!("\x1b[1;{}m{}\x1b[0m", ansi_code, text)
    } else {
        format!("\x1b[{}m{}\x1b[0m", ansi_code, text)
    }
}

/// A SHA-pinned revision abbreviated to the width `git rev-parse --short` uses,
/// so it lines up with the REF column. Branch and tag names pass through.
fn abbreviate_revision(revision: &str) -> String {
    if crate::git::looks_like_sha(revision) && revision.len() > 7 {
        revision[..7].to_string()
    } else {
        revision.to_string()
    }
}

fn print_table(
    statuses: &[RepoStatus],
    orphans: &[crate::resolve::OrphanLink],
    out: &mut dyn Write,
) -> Result<()> {
    if statuses.is_empty() && orphans.is_empty() {
        return Ok(());
    }

    let headers = ["", "REPO", "PATH", "MODE", "REF", "EXPECTED", "STATUS"];
    let mut rows: Vec<[String; 7]> = Vec::new();

    for s in statuses {
        let flags = get_status_flags(s);
        let icon = status_icon(&flags).to_string();
        let ref_str = if s.exists {
            s.current_ref.clone()
        } else {
            "—".to_string()
        };
        let path = if s.is_symlink {
            s.symlink_target.clone()
        } else {
            "-".to_string()
        };
        let mode = s.mode.clone();
        let expected = abbreviate_revision(&s.expected_ref);
        rows.push([
            icon,
            s.directory.clone(),
            path,
            mode,
            ref_str,
            expected,
            flags,
        ]);
    }

    // Append orphaned-symlink rows
    for o in orphans {
        let flags = if o.broken { "orphan, broken" } else { "orphan" }.to_string();
        let icon = status_icon(&flags).to_string();
        rows.push([
            icon,
            o.link_path.display().to_string(),
            "-".to_string(),
            "-".to_string(),
            "—".to_string(),
            String::new(),
            flags,
        ]);
    }

    // Compute column widths
    let mut widths = [0usize; 7];
    for (i, h) in headers.iter().enumerate() {
        widths[i] = h.len();
    }
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }
    // Icon column needs at least 1 (UTF-8 icons are multi-byte but display as 1 char)
    widths[0] = widths[0].max(1);

    let gap = "   ";

    // Print header. The last column is never padded: with something printed
    // after the table, trailing spaces would be real output rather than
    // whitespace the terminal swallows.
    let last = headers.len() - 1;
    let header_line: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| pad(h, if i == last { 0 } else { widths[i] }))
        .collect();
    writeln!(out, "{}", header_line.join(gap))?;

    // Print rows with colors
    for row in &rows {
        let flags = &row[6];
        let color = status_color(flags);
        let icon_bold = !flags.contains("missed");

        let mut cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(i, cell)| pad(cell, if i == last { 0 } else { widths[i] }))
            .collect();

        // Colorize icon (column 0) and status (column 6)
        cells[0] = colorize(&cells[0], color, icon_bold);
        cells[6] = colorize(&cells[6], color, false);

        // Colorize REF (column 4) yellow when it is the wrong ref, which is
        // what STATUS already says. Comparing the two columns as text instead
        // painted every tag-pinned repo yellow for spelling a commit as a
        // commit.
        if flags.contains("ref-mismatch") {
            cells[4] = colorize(&cells[4], "33", false); // yellow
        }

        writeln!(out, "{}", cells.join(gap))?;
    }
    Ok(())
}

fn pad(cell: &str, width: usize) -> String {
    format!("{:<width$}", cell, width = width)
}

fn print_json(
    statuses: &[RepoStatus],
    orphans: &[crate::resolve::OrphanLink],
    cache: Option<&Cache>,
    out: &mut dyn Write,
) -> Result<()> {
    let mut data: Vec<serde_json::Value> = statuses
        .iter()
        .map(|s| {
            serde_json::json!({
                "directory": s.directory,
                "exists": s.exists,
                "current_ref": s.current_ref,
                "expected_ref": s.expected_ref,
                "clean": s.is_clean,
                "detached": s.is_detached,
                "ahead": s.ahead,
                "behind": s.behind,
                "mode": s.mode,
                "stale": s.is_stale,
                "symlink": s.is_symlink,
                "symlink_target": s.symlink_target,
                "cache": s.cache.label(),
            })
        })
        .collect();
    for o in orphans {
        data.push(serde_json::json!({
            "directory": o.link_path.display().to_string(),
            "orphan": true,
            "broken": o.broken,
        }));
    }
    // A row of its own, discriminated by a key, the way orphan rows are —
    // `cache_dir` rather than `cache`, which every repo row above uses for the
    // word in its CACHE column.
    data.push(match cache {
        Some(cache) => {
            let usage = cache.usage();
            serde_json::json!({
                "cache_dir": cache.root().display().to_string(),
                "entries": usage.entries,
                "bytes": usage.bytes,
            })
        }
        None => serde_json::json!({ "cache_dir": null }),
    });
    writeln!(out, "{}", serde_json::to_string_pretty(&data).unwrap())?;
    Ok(())
}

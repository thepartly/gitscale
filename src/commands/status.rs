use anyhow::Result;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::config::{find_config, load_config, load_config_optional, CONFIG_FILENAME};
use crate::git::{fetch_repo, get_artefact_status, get_repo_status, RepoStatus};
use crate::resolve::resolve_recursive;
use crate::storage::fetch_artefact;

pub fn run(
    root: Option<&Path>,
    do_fetch: bool,
    output_format: &str,
    verbose: bool,
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

    if do_fetch {
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
                let _ = fetch_repo(entry, &config_root);
            }
        }
    }

    let mut statuses: Vec<RepoStatus> = Vec::new();

    for entry in &config.repos {
        if entry.is_artefact() {
            statuses.push(get_artefact_status(entry, &config_root));
        } else {
            statuses.push(get_repo_status(entry, &config_root));
        }
    }

    // Check for expected symlinks that are no longer symlinks
    if let Ok((symlinks, _)) = resolve_recursive(&config.repos, &config_root) {
        for sym in &symlinks {
            let link_abs = config_root.join(&sym.link_path);
            if link_abs.exists() && !link_abs.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
                // Find the parent repo that owns this link
                let parent_dir = sym.link_path.iter().next()
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
    }

    if output_format == "json" {
        print_json(&statuses, out)?;
    } else {
        print_table(&statuses, out)?;
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
            parts.first().and_then(|v| v.parse::<i32>().ok()).unwrap_or(0) > 0
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
    if !s.expected_ref.is_empty()
        && s.current_ref != s.expected_ref
        && !s.is_detached
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
    if flags.contains("unlinked") {
        return "~";
    }
    if flags.contains("dirty") {
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
    if flags == "unlinked" {
        return "33"; // yellow
    }
    if flags.contains("dirty") || flags.contains("ref-mismatch") || flags.contains("stale") || flags.contains("unlinked") {
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

fn print_table(statuses: &[RepoStatus], out: &mut dyn Write) -> Result<()> {
    if statuses.is_empty() {
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
        let expected = s.expected_ref.clone();
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

    // Print header
    let header_line: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, h)| format!("{:<width$}", h, width = widths[i]))
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
            .map(|(i, cell)| format!("{:<width$}", cell, width = widths[i]))
            .collect();

        // Colorize icon (column 0) and status (column 6)
        cells[0] = colorize(&cells[0], color, icon_bold);
        cells[6] = colorize(&cells[6], color, false);

        // Colorize REF (column 4) yellow if it differs from EXPECTED (column 5)
        let ref_val = row[4].trim();
        let expected_val = row[5].trim();
        if !ref_val.is_empty() && !expected_val.is_empty() && ref_val != expected_val {
            cells[4] = colorize(&cells[4], "33", false); // yellow
        }

        writeln!(out, "{}", cells.join(gap))?;
    }
    Ok(())
}

fn print_json(statuses: &[RepoStatus], out: &mut dyn Write) -> Result<()> {
    let data: Vec<serde_json::Value> = statuses
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
            })
        })
        .collect();
    writeln!(out, "{}", serde_json::to_string_pretty(&data).unwrap())?;
    Ok(())
}

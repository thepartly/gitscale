use anyhow::Result;
use std::path::Path;

use crate::config::{find_config, load_config};
use crate::git::{fetch_repo, get_artefact_status, get_repo_status, get_self_status, RepoStatus};
use crate::storage::fetch_artefact;

pub fn run(root: Option<&Path>, do_fetch: bool, output_format: &str, verbose: bool) -> Result<()> {
    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;

    if config.repos.is_empty() {
        println!("No repos declared in .gitscale.toml");
        return Ok(());
    }

    if do_fetch {
        for entry in &config.repos {
            let dest = config_root.join(&entry.directory);
            if entry.is_artefact() {
                if !config.storage_url.is_empty() {
                    let revision = if entry.revision.is_empty() { "HEAD" } else { &entry.revision };
                    if verbose {
                        println!("Fetching {}...", entry.directory);
                    }
                    let _ = fetch_artefact(&config.storage_url, &entry.repo_url, revision, &dest);
                }
                continue;
            }
            if dest.exists() {
                if verbose {
                    println!("Fetching {}...", entry.directory);
                }
                let _ = fetch_repo(entry, &config_root);
            }
        }
    }

    let mut statuses: Vec<RepoStatus> = Vec::new();

    if let Some(self_status) = get_self_status(&config_root) {
        statuses.push(self_status);
    }

    for entry in &config.repos {
        if entry.is_artefact() {
            statuses.push(get_artefact_status(entry, &config_root));
        } else {
            statuses.push(get_repo_status(entry, &config_root));
        }
    }

    if output_format == "json" {
        print_json(&statuses);
    } else {
        print_table(&statuses);
    }
    Ok(())
}

fn get_status_flags(s: &RepoStatus) -> String {
    if !s.exists {
        return "missed".to_string();
    }
    let mut flags: Vec<String> = Vec::new();
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
    if flags.contains("missed") {
        return "✘";
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
    if flags.contains("missed") {
        return "31"; // red
    }
    if flags.contains("dirty") || flags.contains("ref-mismatch") || flags.contains("stale") {
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

fn print_table(statuses: &[RepoStatus]) {
    if statuses.is_empty() {
        return;
    }

    let headers = ["", "REPO", "MODE", "REF", "EXPECTED", "STATUS"];
    let mut rows: Vec<[String; 6]> = Vec::new();

    for s in statuses {
        let flags = get_status_flags(s);
        let icon = status_icon(&flags).to_string();
        let ref_str = if s.exists {
            s.current_ref.clone()
        } else {
            "—".to_string()
        };
        rows.push([
            icon,
            s.directory.clone(),
            s.mode.clone(),
            ref_str,
            s.expected_ref.clone(),
            flags,
        ]);
    }

    // Compute column widths
    let mut widths = [0usize; 6];
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
    println!("{}", header_line.join(gap));

    // Print rows with colors
    for row in &rows {
        let flags = &row[5];
        let color = status_color(flags);
        let icon_bold = !flags.contains("missed");

        let mut cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(i, cell)| format!("{:<width$}", cell, width = widths[i]))
            .collect();

        // Colorize icon (column 0) and status (column 5)
        cells[0] = colorize(&cells[0], color, icon_bold);
        cells[5] = colorize(&cells[5], color, false);

        println!("{}", cells.join(gap));
    }
}

fn print_json(statuses: &[RepoStatus]) {
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
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&data).unwrap());
}

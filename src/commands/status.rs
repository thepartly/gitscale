use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::cache::{self, Cache};
use crate::commands::cache::Sources;
use crate::config::load_workspace;
use crate::git::{fetch_repo, get_artefact_status, get_repo_status, is_tree_modified, RepoStatus};
use crate::resolution::{Kind, Resolution, Slot};

#[allow(clippy::too_many_arguments)]
pub fn run(
    root: Option<&Path>,
    do_fetch: bool,
    output_format: &str,
    why: Option<&[String]>,
    verbose: bool,
    no_cache: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;

    // JSON goes on to print its usual array, so a consumer gets JSON either way.
    if config.repos.is_empty() && output_format != "json" && why.is_none() {
        writeln!(out, "No repos declared in .gitscale.toml")?;
        return Ok(());
    }

    // Never `Sources::adopting`: status changes nothing, even with --fetch.
    let sources = Sources::new(&config, &config_root, no_cache, verbose);
    let artefacts = crate::artefact::Artefacts::new(&config, &config_root, sources.cache.clone());

    // Offline unless `--fetch`: what this machine already has. A graph that
    // cannot be resolved is still worth a table — the declared entries, each
    // marked unresolved, under the reason.
    let resolution = match crate::resolve::workspace(
        &config,
        &config_root,
        do_fetch,
        sources.cache.clone(),
        Some(&artefacts),
        verbose,
    ) {
        Ok(resolution) => resolution,
        Err(e) => {
            writeln!(err, "error: {:#}", e)?;
            unresolved(&config, &format!("{:#}", e))
        }
    };

    if let Some(dirs) = why {
        return print_why(&resolution, dirs, out);
    }

    if do_fetch {
        for slot in &resolution.slots {
            let entry = slot.entry();
            let dest = config_root.join(&entry.directory);
            let fetched = if entry.is_artefact() {
                if dest.is_symlink() {
                    Ok(())
                } else {
                    if verbose {
                        writeln!(out, "Fetching {}...", entry.directory)?;
                    }
                    artefacts.fetch(&entry).map(|_| ())
                }
            } else if crate::git::is_checkout(&dest) && !dest.is_symlink() {
                if verbose {
                    writeln!(out, "Fetching {}...", entry.directory)?;
                }
                let from = sources.for_entry(&entry);
                fetch_repo(&entry, &config_root, &from)
            } else {
                Ok(())
            };
            // Status still reports, but must not pass off what the last
            // successful fetch saw as what `--fetch` just found.
            if let Err(e) = fetched {
                writeln!(
                    err,
                    "  fetch {}: {} (showing the last fetched state)",
                    entry.directory, e
                )?;
            }
        }
    }

    let mut statuses: Vec<RepoStatus> = Vec::new();

    // Resolved whether or not the cache is switched on: `--no-cache` changes
    // what the next command does, not where the objects a checkout already
    // borrows happen to live.
    let cache_root = cache::resolve_dir(&config.cache.dir);

    let entries = resolution.entries();
    for entry in &entries {
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

    // The links planted inside a checkout are gitscale's, not its owner's
    // work: they do not make it dirty. Where the repository does not ignore
    // them, they are untracked files all the same, and that is said apart.
    for status in statuses.iter_mut() {
        if !status.exists || status.is_symlink || status.artefact.is_some() {
            continue;
        }
        let planted: Vec<String> = resolution
            .links
            .iter()
            .filter_map(|link| {
                let owner = crate::resolve::owning_entry(&link.link_path, &entries)?;
                (owner.directory == status.directory).then(|| {
                    link.link_path
                        .strip_prefix(&owner.directory)
                        .unwrap_or(&link.link_path)
                        .to_string_lossy()
                        .into_owned()
                })
            })
            .collect();
        if planted.is_empty() || status.is_clean {
            continue;
        }
        let Some(listed) = crate::git::porcelain(&config_root.join(&status.directory)) else {
            continue;
        };
        let (links, rest): (Vec<_>, Vec<_>) = listed
            .into_iter()
            .partition(|(untracked, path)| *untracked && planted.contains(path));
        status.untracked_links = links.into_iter().map(|(_, path)| path).collect();
        status.is_clean = rest.is_empty();
    }

    // Check for expected symlinks that are no longer symlinks
    for sym in &resolution.links {
        let link_abs = config_root.join(&sym.link_path);
        if link_abs.exists()
            && !link_abs
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        {
            let Some(owner) = crate::resolve::owning_entry(&sym.link_path, &entries) else {
                continue;
            };
            if let Some(s) = statuses.iter_mut().find(|s| s.directory == owner.directory) {
                s.has_unlinked = true;
                if is_tree_modified(&link_abs) {
                    s.has_unlinked_modified = true;
                }
            }
        }
    }

    // Collect orphaned symlinks (declared deps that were removed from config)
    let orphans = crate::resolve::find_orphan_links(
        &entries,
        &config_root,
        &resolution.links,
        &config.resolve.hoist_dir,
    );

    let rows: Vec<(&RepoStatus, &Slot)> = statuses.iter().zip(&resolution.slots).collect();
    if output_format == "json" {
        print_json(&rows, &orphans, sources.cache.as_ref(), out)?;
    } else {
        print_table(&rows, &orphans, out)?;
    }
    Ok(())
}

/// The root's entries as declared, every one unresolved: what status shows
/// when the graph as a whole cannot be resolved.
fn unresolved(config: &crate::config::GitScaleConfig, reason: &str) -> Resolution {
    Resolution {
        slots: config
            .repos
            .iter()
            .map(|e| Slot {
                directory: e.directory.clone(),
                url: e.repo_url.clone(),
                mode: e.mode,
                recursive: e.recursive,
                kind: Kind::of(e.mode),
                class: crate::version::Class::Any,
                declared: Some(e.revision.clone()),
                implicit: false,
                chosen: None,
                unresolved: Some(reason.to_string()),
                unread: None,
                requests: Vec::new(),
                majors: 1,
            })
            .collect(),
        links: Vec::new(),
    }
}

/// Is this slot unresolved, as far as the table is concerned? An artefact
/// whose dependencies cannot be read is one whose image is not here, which
/// its own flags already say.
fn is_unresolved(slot: &Slot) -> bool {
    slot.unresolved.is_some() || (slot.unread.is_some() && slot.kind == Kind::Source)
}

/// The RESOLUTION column: how the row's revision was chosen, when there is
/// anything to say beyond "the root asked for it". The winner only, and a
/// count when more than one repository asked — `--why` has the rest.
fn notes(slot: &Slot) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(reason) = &slot.unresolved {
        notes.push(
            if reason.contains("not on this machine") || reason.contains("not been fetched") {
                "not fetched yet: run status --fetch".to_string()
            } else {
                "see the error above".to_string()
            },
        );
    } else if slot.unread.is_some() && slot.kind == Kind::Source {
        notes.push("its dependencies are not fetched yet: run status --fetch".to_string());
    }
    if let Some(chosen) = &slot.chosen {
        if chosen.resolution == "raised" {
            notes.push(format!(
                "raised from {} by {}",
                slot.declared.as_deref().unwrap_or_default(),
                chosen.by
            ));
        }
        if let Some(beaten) = slot.requests.iter().find(|r| r.overruled_by.is_some()) {
            let by = beaten.overruled_by.as_deref().unwrap_or("root");
            let missing = if beaten.missing {
                " (no such revision)"
            } else {
                ""
            };
            let held = if by == "root" {
                format!("held at {}", chosen.revision)
            } else {
                format!("held at {} by {}", chosen.revision, by)
            };
            notes.push(format!(
                "{}, {} wants {}{}",
                held, beaten.from, beaten.revision, missing
            ));
        }
    }
    if let Some(newer) = slot.requests.iter().find(|r| r.ahead) {
        notes.push(format!("behind {}'s {}", newer.from, newer.revision));
    }
    if slot.implicit {
        notes.push(format!(
            "implicit via {}",
            slot.first_requester().unwrap_or("root")
        ));
    }
    if slot.majors > 1 {
        notes.push(format!("{} majors", slot.majors));
    }
    if slot.requests.len() > 1 {
        notes.push(format!("{} requests", slot.requests.len()));
    }
    notes
}

/// STATUS for a row: its own flags, then resolution's warnings — `override`
/// and `unresolved` — with a plain `ok` folded away when there is one.
fn row_flags(s: &RepoStatus, slot: &Slot) -> String {
    let flags = get_status_flags(s);
    // Nothing there yet: `missed` says all there is, and the revision is the
    // one `clone` will resolve.
    if flags == "missed" {
        return flags;
    }
    let mut warnings = Vec::new();
    if is_unresolved(slot) {
        warnings.push("unresolved");
    }
    if slot.requests.iter().any(|r| r.overruled_by.is_some()) {
        warnings.push("override");
    }
    if warnings.is_empty() {
        return flags;
    }
    if flags == "ok" {
        return warnings.join(", ");
    }
    format!("{}, {}", flags, warnings.join(", "))
}

/// `gitscale status --why`: how each slot got its revision. With no
/// directories, every slot more than one repository asks for.
fn print_why(resolution: &Resolution, dirs: &[String], out: &mut dyn Write) -> Result<()> {
    let slots: Vec<&Slot> = if dirs.is_empty() {
        resolution
            .slots
            .iter()
            .filter(|s| s.requests.len() > 1)
            .collect()
    } else {
        dirs.iter()
            .map(|d| {
                resolution
                    .slot(d.trim_end_matches('/'))
                    .ok_or_else(|| anyhow::anyhow!("no checkout at {}", d))
            })
            .collect::<Result<_>>()?
    };
    if slots.is_empty() {
        writeln!(out, "Every dependency is asked for by one repository only.")?;
        return Ok(());
    }
    for (i, slot) in slots.iter().enumerate() {
        if i > 0 {
            writeln!(out)?;
        }
        let class = match slot.class {
            crate::version::Class::Any => "any version".to_string(),
            class => format!("major {}", class),
        };
        writeln!(
            out,
            "{}  {}  {}  {}  {}",
            slot.directory,
            slot.url,
            class,
            slot.kind.label(),
            slot.mode
        )?;
        match (&slot.chosen, &slot.unresolved) {
            (_, Some(reason)) => writeln!(out, "  unresolved  {}", reason)?,
            (Some(chosen), None) => writeln!(
                out,
                "  selected  {} ({})  {}, {}",
                chosen.revision,
                crate::git::short_sha(&chosen.commit),
                chosen.kind.label(),
                chosen.reason.describe()
            )?,
            (None, None) => writeln!(
                out,
                "  selected  no revision asked for: follows the branch its checkout is on"
            )?,
        }
        if slot.requests.is_empty() {
            continue;
        }
        writeln!(out, "  requests")?;
        let width = slot
            .requests
            .iter()
            .map(|r| r.path().chars().count())
            .max()
            .unwrap_or(0);
        let rev_width = slot
            .requests
            .iter()
            .map(|r| r.revision.chars().count().max(1))
            .max()
            .unwrap_or(1);
        for request in &slot.requests {
            let mut marks = Vec::new();
            if let Some(kind) = request.revision_kind {
                marks.push(kind.label().to_string());
            }
            if request.is_override {
                marks.push("override".to_string());
            }
            if request.selected {
                marks.push("selected".to_string());
            }
            if request.missing {
                marks.push("no such revision".to_string());
            }
            if let Some(by) = &request.overruled_by {
                marks.push(format!("overruled by {}", by));
            }
            let revision = if request.revision.is_empty() {
                "-"
            } else {
                request.revision.as_str()
            };
            let line = format!(
                "    {}  {}  {}",
                pad(&request.path(), width),
                pad(revision, rev_width),
                marks.join("   ")
            );
            writeln!(out, "{}", line.trim_end())?;
        }
    }
    Ok(())
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
    // An artefact has no working tree to be dirty and no history to count:
    // what there is to say is how the installed commit compares with the
    // configured revision and with what the last fetch saw.
    if let Some(artefact) = &s.artefact {
        flags.extend(artefact.flags.iter().cloned());
        return if flags.is_empty() {
            "ok".to_string()
        } else {
            flags.join(", ")
        };
    }
    if !s.is_clean {
        flags.push("dirty".to_string());
    }
    if !s.untracked_links.is_empty() {
        flags.push("untracked-links".to_string());
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
    {
        flags.push("ref-mismatch".to_string());
    }
    if flags.is_empty() {
        "ok".to_string()
    } else {
        flags.join(", ")
    }
}

fn status_icon(flags: &str) -> &'static str {
    if flags == "ok" || flags.starts_with("ok, ") {
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
    if flags.contains("dirty")
        || flags.contains("cache-broken")
        || flags.contains("missing")
        || flags.contains("changed")
        || flags.contains("untracked-links")
    {
        return "!";
    }
    if flags.contains("stale") || flags.contains("ref-mismatch") {
        return "≠";
    }
    // Resolution's own warnings, ahead of a checkout's ordinary state.
    if flags.contains("unresolved") {
        return "?";
    }
    if flags.contains("override") {
        return "↧";
    }
    if flags.contains("behind") {
        return "⇓";
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

fn status_color(flags: &str) -> &'static str {
    if flags == "ok" || flags.starts_with("ok, ") {
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
        || flags.contains("missing")
        || flags.contains("changed")
    {
        return "91"; // bright red
    }
    if flags.contains('+')
        || flags.contains("untracked-links")
        || flags.contains('-')
        || flags.contains("behind")
        || flags.contains("unresolved")
        || flags.contains("override")
    {
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

/// A SHA-pinned revision abbreviated the way the REF column spells a detached
/// HEAD, so the two line up. Branch and tag names pass through.
fn abbreviate_revision(revision: &str) -> String {
    if crate::git::is_full_sha(revision) {
        crate::git::short_sha(revision).to_string()
    } else {
        revision.to_string()
    }
}

fn print_table(
    rows_in: &[(&RepoStatus, &Slot)],
    orphans: &[crate::resolve::OrphanLink],
    out: &mut dyn Write,
) -> Result<()> {
    if rows_in.is_empty() && orphans.is_empty() {
        return Ok(());
    }

    // RESOLUTION only when some row has something to say: a workspace where
    // the root decides everything reads as it always has.
    let resolutions: Vec<String> = rows_in
        .iter()
        .map(|(s, slot)| {
            // Nothing on disk yet: `missed` says all there is.
            if get_status_flags(s) == "missed" {
                String::new()
            } else {
                notes(slot).join(", ")
            }
        })
        .collect();
    let with_resolution = resolutions.iter().any(|r| !r.is_empty());
    let mut headers = vec!["", "REPO", "PATH", "MODE", "REF", "EXPECTED", "STATUS"];
    if with_resolution {
        headers.push("RESOLUTION");
    }
    // Each row, and the icon and colour its flags give it.
    let mut rows: Vec<(Vec<String>, &str)> = Vec::new();

    for ((s, slot), resolution) in rows_in.iter().zip(&resolutions) {
        let flags = row_flags(s, slot);
        // A winner behind what a losing request asked for is a warning too,
        // though not a state of the checkout: said in RESOLUTION, shown here.
        let behind = flags == "ok" && slot.requests.iter().any(|r| r.ahead);
        let (icon, color) = if behind {
            ("↧", "33")
        } else {
            (status_icon(&flags), status_color(&flags))
        };
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
        let mut row = vec![
            icon.to_string(),
            s.directory.clone(),
            path,
            s.mode.clone(),
            ref_str,
            abbreviate_revision(&s.expected_ref),
            flags,
        ];
        if with_resolution {
            row.push(resolution.clone());
        }
        rows.push((row, color));
    }

    // Append orphaned-symlink rows
    for o in orphans {
        let flags = if o.broken { "orphan, broken" } else { "orphan" }.to_string();
        let color = status_color(&flags);
        let mut row = vec![
            status_icon(&flags).to_string(),
            o.link_path.display().to_string(),
            "-".to_string(),
            "-".to_string(),
            "—".to_string(),
            String::new(),
            flags,
        ];
        if with_resolution {
            row.push(String::new());
        }
        rows.push((row, color));
    }

    // Column widths in characters, the unit `{:<width$}` pads in: counted in
    // bytes, every multi-byte icon and dash widened its column.
    let mut widths = vec![0usize; headers.len()];
    for (i, h) in headers.iter().enumerate() {
        widths[i] = h.chars().count();
    }
    for (row, _) in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }

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
    for (row, color) in &rows {
        let flags = &row[6];
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

    // More than one repository asked for something: the full story is one
    // command away.
    if rows_in.iter().any(|(_, slot)| slot.requests.len() > 1) {
        writeln!(
            out,
            "hint: gitscale status --why <dir> lists every request behind a revision"
        )?;
    }

    // What to do about links git sees: ignore the directories they live in.
    for (s, _) in rows_in {
        if s.untracked_links.is_empty() {
            continue;
        }
        let mut dirs: Vec<String> = s
            .untracked_links
            .iter()
            .map(|link| match std::path::Path::new(link).parent() {
                Some(parent) if !parent.as_os_str().is_empty() => {
                    format!("/{}/", parent.display())
                }
                _ => format!("/{}", link),
            })
            .collect();
        dirs.sort();
        dirs.dedup();
        writeln!(
            out,
            "hint: {}: the dependency links gitscale planted are untracked files there; \
             add {} to its .gitignore",
            s.directory,
            dirs.join(" and ")
        )?;
    }
    Ok(())
}

fn pad(cell: &str, width: usize) -> String {
    format!("{:<width$}", cell, width = width)
}

fn print_json(
    rows: &[(&RepoStatus, &Slot)],
    orphans: &[crate::resolve::OrphanLink],
    cache: Option<&Cache>,
    out: &mut dyn Write,
) -> Result<()> {
    let mut data: Vec<serde_json::Value> = rows
        .iter()
        .map(|(s, slot)| {
            let mut row = serde_json::json!({
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
                "untracked_links": s.untracked_links,
                "cache": s.cache.label(),
            });
            add_resolution(&mut row, slot);
            // What an artefact has installed, and what the last fetch saw for
            // its revision — the commit and image digest of each.
            if let Some(artefact) = &s.artefact {
                let side = |m: &Option<crate::artefact::Marker>| match m {
                    Some(m) => serde_json::json!({"commit": m.commit, "digest": m.digest}),
                    None => serde_json::Value::Null,
                };
                row["artefact"] = serde_json::json!({
                    "installed": side(&artefact.installed),
                    "remote": side(&artefact.remote),
                    "flags": artefact.flags,
                });
            }
            row
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

/// How resolution got to the row's revision: structure only, for consumers
/// that decide for themselves what to make of it.
fn add_resolution(row: &mut serde_json::Value, slot: &Slot) {
    let chosen = slot.chosen.as_ref();
    let requests: Vec<serde_json::Value> = slot
        .requests
        .iter()
        .map(|r| {
            serde_json::json!({
                "from": r.from,
                "chain": r.chain,
                "directory": r.directory,
                "revision": r.revision,
                "revision_kind": r.revision_kind.map(|k| k.label()),
                "override": r.is_override,
                "selected": r.selected,
                "overruled_by": r.overruled_by,
                "ahead": r.ahead,
                "missing": r.missing,
            })
        })
        .collect();
    let resolution = match (chosen, &slot.unresolved) {
        (_, Some(_)) => "unresolved",
        (Some(chosen), None) => chosen.resolution,
        (None, None) => "follow",
    };
    let fields = serde_json::json!({
        "declared_ref": slot.declared.clone().unwrap_or_default(),
        "resolved_ref": chosen.map(|c| c.revision.clone()).unwrap_or_default(),
        "resolved_commit": chosen.map(|c| c.commit.clone()),
        // Nobody gave a revision: the checkout follows a branch.
        "revision_kind": chosen.map_or(
            if slot.unresolved.is_some() { None } else { Some("branch") },
            |c| Some(c.kind.label()),
        ),
        "class": slot.class.describe(),
        "kind": slot.kind.label(),
        "implicit": slot.implicit,
        "resolution": resolution,
        "reason": chosen.map(|c| c.reason.label()),
        "unresolved": slot.unresolved,
        "unread": slot.unread,
        "notes": notes(slot),
        "requests": requests,
    });
    if let (Some(row), serde_json::Value::Object(fields)) = (row.as_object_mut(), fields) {
        row.extend(fields);
    }
}

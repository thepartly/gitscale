use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::config::load_workspace;
use crate::git::{get_artefact_status, get_repo_status, is_tree_modified, Expected, RepoStatus};
use crate::promote::State;
use crate::resolution::{Kind, Resolution, Slot};
use crate::store::Sources;

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

    let sources = Sources::new(&config_root, no_cache)?;
    let artefacts = crate::artefact::Artefacts::new(&config, &config_root, sources.images());

    // Offline unless `--fetch`: what this machine already has. A graph that
    // cannot be resolved is still worth a table — the declared entries, each
    // marked unresolved, under the reason.
    let resolution = match crate::resolve::workspace(
        &config,
        &config_root,
        do_fetch,
        &sources,
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

    // Resolving online fetched every store; what is left is what the
    // registry has for each artefact.
    if do_fetch {
        for slot in &resolution.slots {
            let entry = slot.entry();
            let dest = config_root.join(&entry.directory);
            if !entry.is_artefact() || dest.is_symlink() {
                continue;
            }
            if verbose {
                writeln!(out, "Fetching {}...", entry.directory)?;
            }
            // Status still reports, but must not pass off what the last
            // successful fetch saw as what `--fetch` just found.
            if let Err(e) = artefacts.fetch(&entry) {
                writeln!(
                    err,
                    "  fetch {}: {} (showing the last fetched state)",
                    entry.directory, e
                )?;
            }
        }
    }

    let mut statuses: Vec<RepoStatus> = Vec::new();
    let entries = resolution.entries();
    for (entry, slot) in entries.iter().zip(&resolution.slots) {
        let dest = config_root.join(&entry.directory);
        // An artefact on the topic may be its source instead.
        if entry.is_artefact() && !crate::git::is_checkout(&dest) {
            statuses.push(get_artefact_status(entry, &config_root));
            continue;
        }
        let expected = Expected {
            branch: slot
                .topic
                .as_ref()
                .filter(|t| t.developed || !entry.is_artefact())
                .map(|t| t.branch.clone()),
            commit: slot
                .topic
                .as_ref()
                .map_or(slot.commit.clone(), |t| Some(t.commit.clone())),
        };
        let mut status = get_repo_status(entry, &config_root, &expected);
        if status.exists && !status.is_symlink {
            if let Some(stores) = &sources.stores {
                let store = stores.repo_path(&crate::git::remote_url(entry));
                status.foreign = !crate::store::is_worktree_of(&store, &dest);
            }
        }
        statuses.push(status);
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

    let topic = topic_view(&config, &config_root, &sources, &resolution);
    let warnings = warnings(&config_root);
    let rows: Vec<(&RepoStatus, &Slot)> = statuses.iter().zip(&resolution.slots).collect();
    if output_format == "json" {
        print_json(&rows, &orphans, &topic, &warnings, out)?;
    } else {
        for warning in &warnings {
            writeln!(out, "warning: {}", warning)?;
        }
        print_table(&rows, &orphans, &topic, out)?;
    }
    Ok(())
}

/// What the table says about the workspace's topic: its branch, where each
/// topic slot's change stands, what it waits on, and what may merge next.
/// Worked out offline, from the root's stores: never in CI.
#[derive(Default)]
struct TopicView {
    branch: Option<String>,
    states: std::collections::BTreeMap<String, State>,
    waits: std::collections::BTreeMap<String, Vec<String>>,
    /// Topic slots whose branch lacks the revision the graph now pins:
    /// `behind <revision> wanted by <requester>`.
    behind: std::collections::BTreeMap<String, String>,
    next: Vec<String>,
}

fn topic_view(
    config: &crate::config::GitScaleConfig,
    config_root: &Path,
    sources: &Sources,
    resolution: &Resolution,
) -> TopicView {
    let Some(branch) = crate::topic::root(config, config_root, false)
        .topic()
        .map(str::to_string)
    else {
        return TopicView::default();
    };
    let Some(stores) = &sources.stores else {
        return TopicView {
            branch: Some(branch),
            ..TopicView::default()
        };
    };
    let mut states = std::collections::BTreeMap::new();
    let mut behind = std::collections::BTreeMap::new();
    for slot in resolution.topic_slots() {
        let store = stores.repo_path(&crate::ci::remote_url(&slot.url));
        let planted = resolution.planted_in(&slot.directory);
        let state = crate::promote::assess(config_root, &store, slot, &planted, None);
        states.insert(slot.directory.clone(), state);
        let (Some(on), Some(pin)) = (&slot.topic, slot.pin()) else {
            continue;
        };
        let contains = crate::git::run_git(
            &["merge-base", "--is-ancestor", &pin.commit, &on.commit],
            Some(&store),
            false,
        );
        if contains.is_ok_and(|o| o.status.code() == Some(1)) {
            behind.insert(
                slot.directory.clone(),
                format!("behind {} wanted by {}: rebase it", pin.revision, pin.by),
            );
        }
    }
    let promoted: std::collections::BTreeSet<String> = states
        .iter()
        .filter(|(_, s)| s.is_promoted())
        .map(|(d, _)| d.clone())
        .collect();
    let requests = crate::promote::topic_requests(config, config_root, resolution);
    let waits = states
        .keys()
        .map(|dir| {
            (
                dir.clone(),
                crate::promote::waits_on(&requests, &promoted, dir),
            )
        })
        .collect();
    let next = if states.is_empty() {
        Vec::new()
    } else {
        crate::promote::next_to_merge(&requests, &promoted)
    };
    TopicView {
        branch: Some(branch),
        states,
        waits,
        behind,
        next,
    }
}

/// What is wrong with the workspace as a whole, rather than with one row.
fn warnings(config_root: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    // `git clone --bare` sets none: `origin/*` is then never updated.
    let refspec = crate::git::query(config_root, &["config", "--get-all", "remote.origin.fetch"]);
    if crate::git::origin_url(config_root).is_some() && refspec.is_none_or(|r| r.is_empty()) {
        warnings.push(
            "the root repository has no fetch refspec, so origin/* is never updated\n  \
             ahead/behind and upstreams in this table may be wrong. To fix:\n  \
             git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*' && git fetch"
                .to_string(),
        );
    }
    warnings
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
                artefact: e.artefact,
                recursive: e.recursive,
                kind: Kind::of(e),
                class: crate::version::Class::Any,
                declared: Some(e.revision.clone()),
                implicit: false,
                chosen: None,
                unresolved: Some(reason.to_string()),
                unread: None,
                requests: Vec::new(),
                majors: 1,
                commit: None,
                branch: None,
                topic: None,
                pinned_by: None,
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
fn notes(slot: &Slot, topic: &TopicView, status: Option<&RepoStatus>) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(on) = &slot.topic {
        notes.push(if on.developed {
            "topic".to_string()
        } else {
            "topic, from remote".to_string()
        });
        if slot.artefact == Some(crate::config::ArtefactUse::Replace) {
            let sources = status.is_some_and(|s| s.artefact.is_none());
            notes.push(if sources {
                "sources".to_string()
            } else {
                format!("image {}", crate::git::short_sha(&on.commit))
            });
        }
        if let Some(waits) = topic.waits.get(&slot.directory).filter(|w| !w.is_empty()) {
            notes.push(format!("waits on {}", waits.join(", ")));
        }
        if let Some(behind) = topic.behind.get(&slot.directory) {
            notes.push(behind.clone());
        }
        if let Some(state) = topic.states.get(&slot.directory) {
            if !matches!(state, State::Unknown(_)) {
                notes.push(state.describe());
            }
        }
    }
    if let Some(by) = &slot.pinned_by {
        notes.push(format!("pinned by {}", by));
    }
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
    if let Some(chosen) = slot.chosen.as_ref().filter(|_| slot.topic.is_none()) {
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
            "{}  {}  {}  {}{}",
            slot.directory,
            slot.url,
            class,
            slot.kind.label(),
            slot.artefact
                .map(|a| format!("  {}", a))
                .unwrap_or_default()
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
                "  selected  no revision asked for: the default branch's head"
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
    // Not a worktree of the root's store: a clone an older gitscale made, or
    // one whose store is gone. `pull` will not touch it.
    if s.foreign {
        flags.push("foreign".to_string());
    }
    if s.is_stale {
        flags.push("stale".to_string());
    }
    if s.ahead > 0 {
        flags.push(format!("+{}", s.ahead));
    }
    if s.behind > 0 {
        flags.push(format!("-{}", s.behind));
    }
    // A checkout off the topic is detached, so REF reads as a commit and can
    // never equal the revision as text. What decides it is where HEAD actually
    // is: at the commit resolution selected, or on the topic branch.
    if !s.at_expected {
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
        || flags.contains("foreign")
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
        || flags.contains("foreign")
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
    topic: &TopicView,
    out: &mut dyn Write,
) -> Result<()> {
    if rows_in.is_empty() && orphans.is_empty() {
        return Ok(());
    }
    if let Some(branch) = &topic.branch {
        if topic.next.is_empty() {
            writeln!(out, "topic {}", branch)?;
        } else {
            writeln!(
                out,
                "topic {} · next to merge: {}",
                branch,
                topic.next.join(", ")
            )?;
        }
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
                notes(slot, topic, Some(s)).join(", ")
            }
        })
        .collect();
    let with_resolution = resolutions.iter().any(|r| !r.is_empty());
    let mut headers = vec!["", "REPO", "PATH", "ARTEFACT", "REF", "EXPECTED", "STATUS"];
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
            s.artefact_use.clone(),
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
    topic: &TopicView,
    warnings: &[String],
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
                "artefact_use": s.artefact_use,
                "stale": s.is_stale,
                "symlink": s.is_symlink,
                "symlink_target": s.symlink_target,
                "untracked_links": s.untracked_links,
                "foreign": s.foreign,
            });
            add_resolution(&mut row, slot, topic, Some(s));
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
    // Rows of their own, discriminated by a key, the way orphan rows are.
    if let Some(branch) = &topic.branch {
        data.push(serde_json::json!({ "topic": branch, "next_to_merge": topic.next }));
    }
    if !warnings.is_empty() {
        data.push(serde_json::json!({ "warnings": warnings }));
    }
    writeln!(out, "{}", serde_json::to_string_pretty(&data).unwrap())?;
    Ok(())
}

/// How resolution got to the row's revision: structure only, for consumers
/// that decide for themselves what to make of it.
fn add_resolution(
    row: &mut serde_json::Value,
    slot: &Slot,
    topic: &TopicView,
    status: Option<&RepoStatus>,
) {
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
                "commit": r.commit,
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
        "notes": notes(slot, topic, status),
        "requests": requests,
        "topic": slot.topic.as_ref().map(|t| serde_json::json!({
            "branch": t.branch,
            "commit": t.commit,
            "developed": t.developed,
            "pin": t.pin.as_ref().map(|p| serde_json::json!({
                "revision": p.revision,
                "commit": p.commit,
                "by": p.by,
            })),
            "state": topic.states.get(&slot.directory).map(|s| s.label()),
        })),
        "pinned_by": slot.pinned_by,
    });
    if let (Some(row), serde_json::Value::Object(fields)) = (row.as_object_mut(), fields) {
        row.extend(fields);
    }
}

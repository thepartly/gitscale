//! `git topic`: topics as commands. A topic is one branch name across the
//! repositories a change touches; the root's current branch is the topic,
//! unless the root pins it.
//!
//! * `git topic` prints the topic of the checkout the current directory is
//!   in, for scripts and prompts.
//! * `join` / `leave` put checkouts on the topic and take them off it.
//! * `start`, `switch`, `list` and `finish` begin, go to, show and end
//!   topics: a branch of a plain clone, or a worktree of its own when the
//!   root is a bare repository with worktrees.
//! * `status` shows what the current topic still needs.
//!
//! Every git command here runs with gitscale's hook held off, and the
//! placement it would have run is run here instead: what a topic command
//! leaves behind does not depend on whether, or which, hook is installed.

use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::artefact::Artefacts;
use crate::checkout::Placer;
use crate::config::{load_config, GitScaleConfig, CONFIG_FILENAME};
use crate::paths::{relative_to, Here};
use crate::promote::State;
use crate::resolution::{Resolution, Slot};
use crate::resolve::Network;
use crate::store::Sources;
use crate::topic::Root;

/// What `git topic` was asked to do.
pub enum Action {
    Print,
    Join {
        dirs: Vec<String>,
        dependants: bool,
    },
    Leave(Vec<String>),
    Start {
        name: String,
        from: Option<String>,
        worktree: Option<bool>,
        dir: Option<PathBuf>,
    },
    Switch {
        name: String,
        worktree: Option<bool>,
        dir: Option<PathBuf>,
    },
    Status {
        fetch: bool,
        format: String,
    },
    List {
        fetch: bool,
        format: String,
    },
    Finish {
        name: Option<String>,
        force: bool,
    },
}

pub fn run(
    start: Option<&Path>,
    action: Action,
    verbose: bool,
    no_cache: bool,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let ctx = Ctx {
        start,
        verbose,
        no_cache,
        interactive,
    };
    match action {
        Action::Print => print(&ctx, out, err),
        Action::Join { dirs, dependants } => {
            let how = if dependants {
                How::Dependants
            } else {
                How::Join
            };
            join_or_leave(&ctx, &dirs, how, out, err)
        }
        Action::Leave(dirs) => join_or_leave(&ctx, &dirs, How::Leave, out, err),
        Action::Start {
            name,
            from,
            worktree,
            dir,
        } => start_topic(
            &ctx,
            &name,
            from.as_deref(),
            worktree,
            dir.as_deref(),
            out,
            err,
        ),
        Action::Switch {
            name,
            worktree,
            dir,
        } => switch(&ctx, &name, worktree, dir.as_deref(), out, err),
        Action::Status { fetch, format } => status(&ctx, fetch, &format, out),
        Action::List { fetch, format } => list(&ctx, fetch, &format, out),
        Action::Finish { name, force } => finish(&ctx, name.as_deref(), force, out, err),
    }
}

struct Ctx<'a> {
    start: Option<&'a Path>,
    verbose: bool,
    no_cache: bool,
    interactive: bool,
}

impl Ctx<'_> {
    fn cwd(&self) -> Result<PathBuf> {
        Ok(match self.start {
            Some(p) => p
                .canonicalize()
                .with_context(|| format!("cannot resolve start path {}", p.display()))?,
            None => std::env::current_dir()?,
        })
    }

    /// Place the workspace at `root`, online.
    fn place(&self, root: &Path, out: &mut dyn Write, err: &mut dyn Write) -> Result<()> {
        let config = load_config(&root.join(CONFIG_FILENAME))?;
        crate::commands::sync::place(
            &config,
            root,
            &crate::commands::sync::Placement {
                dirs: &[],
                network: Network::Online,
                force: false,
                leave: None,
                heading: "",
            },
            self.verbose,
            self.no_cache,
            self.interactive,
            out,
            err,
        )
    }
}

// ---------------------------------------------------------------------------
// Where a topic command runs
// ---------------------------------------------------------------------------

/// The repository a topic command works on: a worktree of the root, or —
/// for `start`, `switch`, `list` and `finish` — the directory holding a bare
/// repository and its worktrees.
struct Repo {
    /// Where git runs: the worktree, or the bare repository's parent.
    dir: PathBuf,
    /// The workspace in that worktree; `None` in the bare repository's
    /// parent.
    workspace: Option<(GitScaleConfig, PathBuf)>,
    /// The root's common dir is a bare repository: topics are worktrees.
    bare: bool,
    /// The common git dir.
    common: PathBuf,
}

impl Repo {
    fn find(ctx: &Ctx) -> Result<Repo> {
        let repo = Self::locate(ctx)?;
        // A worktree whose directory was deleted by hand is still registered
        // with git, which keeps its branch from every other worktree: gone
        // to everything here once pruned. Its branches stay.
        let _ = crate::git::run_git(&["worktree", "prune"], Some(&repo.common), false);
        Ok(repo)
    }

    fn locate(ctx: &Ctx) -> Result<Repo> {
        // The directory holding a bare repository at `.git` and its
        // worktrees beside it. Asked first: walking up from it could find
        // another workspace altogether.
        let cwd = ctx.cwd()?;
        let dot_git = cwd.join(".git");
        if dot_git.is_dir() && is_bare(&cwd) {
            return Ok(Repo {
                dir: cwd,
                workspace: None,
                bare: true,
                common: dot_git,
            });
        }
        let (config, root) = crate::config::load_workspace(ctx.start)?;
        let common = crate::git::common_dir(&root)
            .ok_or_else(|| anyhow::anyhow!("{} is not in a git repository", root.display()))?;
        let bare = is_bare(&root);
        Ok(Repo {
            dir: root.clone(),
            workspace: Some((config, root)),
            bare,
            common,
        })
    }

    /// The workspace, for a command that needs a worktree.
    fn workspace(&self) -> Result<(&GitScaleConfig, &Path)> {
        match &self.workspace {
            Some((config, root)) => Ok((config, root)),
            None => bail!(
                "this needs a worktree: run it inside one, or git topic switch <branch> first"
            ),
        }
    }

    /// `[topic] prefix`, `{user}` filled in when `user` asks for it. In the
    /// bare repository's parent, read from the default branch.
    fn prefix(&self, need_user: bool) -> Result<Option<String>> {
        let configured = match &self.workspace {
            Some((config, _)) => config.topic.prefix.clone(),
            None => crate::config::committed_at(&self.dir, "HEAD")
                .and_then(|text| {
                    crate::config::parse_config(&text, Path::new(CONFIG_FILENAME)).ok()
                })
                .and_then(|c| c.topic.prefix),
        };
        match crate::topic::prefix(configured.as_deref(), &self.dir) {
            Ok(prefix) => Ok(prefix),
            Err(e) if need_user => Err(e),
            Err(_) => Ok(None),
        }
    }

    /// `[branches] pinned`, as `prefix` reads it.
    fn pinned(&self) -> Option<Vec<String>> {
        match &self.workspace {
            Some((config, _)) => config.branches.pinned.clone(),
            None => crate::config::committed_at(&self.dir, "HEAD")
                .and_then(|text| {
                    crate::config::parse_config(&text, Path::new(CONFIG_FILENAME)).ok()
                })
                .and_then(|c| c.branches.pinned),
        }
    }

    fn git(&self, args: &[&str]) -> Result<std::process::Output> {
        crate::git::run_git(args, Some(&self.dir), true)
    }

    fn query(&self, args: &[&str]) -> Option<String> {
        crate::git::query(&self.dir, args)
    }

    fn has_ref(&self, name: &str) -> bool {
        crate::git::ref_exists(&self.dir, name)
    }

    /// A bare clone fetches no remote branches until told to: the first
    /// topic command sets that up, and says so.
    fn ensure_fetch_refspec(&self, out: &mut dyn Write) -> Result<()> {
        if !self.bare || crate::git::origin_url(&self.dir).is_none() {
            return Ok(());
        }
        let refspec = self.query(&["config", "--get-all", "remote.origin.fetch"]);
        if refspec.is_some_and(|r| !r.is_empty()) {
            return Ok(());
        }
        self.git(&[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ])?;
        self.fetch()?;
        writeln!(out, "configured origin to fetch remote branches")?;
        Ok(())
    }

    fn fetch(&self) -> Result<()> {
        if crate::git::origin_url(&self.dir).is_none() {
            return Ok(());
        }
        self.git(&["fetch", "--quiet", "--prune", "origin"])
            .context("cannot fetch the root")?;
        Ok(())
    }

    /// The remote's default branch, asking the remote when the clone does
    /// not say.
    fn default_branch(&self) -> Option<String> {
        crate::topic::default_branch(&self.dir, true).or_else(|| {
            // A bare clone's HEAD names the default branch it was cloned at.
            let head = self.query(&["symbolic-ref", "--quiet", "--short", "HEAD"])?;
            self.has_ref(&format!("refs/remotes/origin/{}", head))
                .then_some(head)
        })
    }

    /// Every worktree of the root and the branch it is on.
    fn worktrees(&self) -> Vec<(PathBuf, Option<String>)> {
        let Some(listing) = self.query(&["worktree", "list", "--porcelain"]) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        let mut path: Option<PathBuf> = None;
        let mut branch: Option<String> = None;
        let mut bare = false;
        let flush = |found: &mut Vec<_>,
                     path: &mut Option<PathBuf>,
                     branch: &mut Option<String>,
                     bare: &mut bool| {
            if let Some(p) = path.take() {
                if !*bare {
                    found.push((p.canonicalize().unwrap_or(p), branch.take()));
                }
            }
            *branch = None;
            *bare = false;
        };
        for line in listing.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                flush(&mut found, &mut path, &mut branch, &mut bare);
                path = Some(PathBuf::from(p));
            } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
                branch = Some(b.to_string());
            } else if line == "bare" {
                bare = true;
            }
        }
        flush(&mut found, &mut path, &mut branch, &mut bare);
        found
    }

    fn worktree_on(&self, branch: &str) -> Option<PathBuf> {
        self.worktrees()
            .into_iter()
            .find(|(_, b)| b.as_deref() == Some(branch))
            .map(|(p, _)| p)
    }

    /// Where a new topic's worktree goes: beside the others for a bare
    /// repository at `app/.git` (`app/NAME`), beside the clone for a plain
    /// one (`app-NAME`).
    fn worktree_dir(&self, branch: &str, prefix: Option<&str>) -> PathBuf {
        let name = crate::topic::worktree_name(branch, prefix);
        let parent = self.common.parent().unwrap_or(&self.common);
        if self.bare {
            parent.join(name)
        } else {
            let clone = parent.file_name().unwrap_or_default().to_string_lossy();
            parent
                .parent()
                .unwrap_or(parent)
                .join(format!("{}-{}", clone, name))
        }
    }

    /// Whether topics are worktrees here: as asked, else the person's own
    /// `gitscale.topic.worktree`, else the layout.
    fn use_worktrees(&self, asked: Option<bool>) -> bool {
        if let Some(asked) = asked {
            return asked;
        }
        match self
            .query(&["config", "--type=bool", "--get", "gitscale.topic.worktree"])
            .as_deref()
        {
            Some("true") => true,
            Some("false") => false,
            _ => self.bare,
        }
    }

    fn is_pinned(&self, branch: &str) -> bool {
        crate::topic::is_pinned(
            branch,
            self.pinned().as_deref(),
            self.default_branch().as_deref(),
        )
    }
}

/// Whether the repository at `dir` shares a bare common dir.
fn is_bare(dir: &Path) -> bool {
    crate::git::query(dir, &["config", "--type=bool", "--get", "core.bare"]).as_deref()
        == Some("true")
}

fn refuse_in_ci(what: &str) -> Result<()> {
    if crate::git::is_ci() {
        bail!("git topic {} works on a developer machine, not in CI", what);
    }
    Ok(())
}

/// Offline at a terminal, the line that says how old what was read is, when
/// an hour or more: a merge or a release since is not in it.
fn offline_age(ctx: &Ctx, repos: &[PathBuf], command: &str, out: &mut dyn Write) -> Result<()> {
    if !ctx.interactive {
        return Ok(());
    }
    if let Some(age) = crate::store::fetched_long_ago(repos) {
        writeln!(
            out,
            "{}",
            crate::output::hint(
                &format!(
                    "fetched {} ago: {} fetches first without --offline",
                    age, command
                ),
                crate::output::stdout()
            )
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// git topic
// ---------------------------------------------------------------------------

/// The topic branch of the checkout the current directory is in; nothing,
/// and exit 1, off a topic. Offline. A person at a terminal is told where
/// they are instead, on stderr: stdout stays what `$(git topic)` reads.
fn print(ctx: &Ctx, out: &mut dyn Write, err: &mut dyn Write) -> Result<()> {
    let (config, root) = crate::config::load_workspace(ctx.start)?;
    let mut no_topic = |why: &str| -> Result<()> {
        if ctx.interactive {
            writeln!(err, "{}", why)?;
        }
        Err(crate::reported())
    };
    let topic = match crate::topic::root(&config, &root, false) {
        Root::Topic(topic) => topic,
        Root::Pinned(branch) => {
            return no_topic(&format!(
                "{} is pinned, not a topic: git topic start NAME, or git topic switch NAME",
                branch
            ))
        }
        Root::Detached => {
            return no_topic(
                "on no branch, not a topic: git topic start NAME, or git topic switch NAME",
            )
        }
    };
    let here = Here::new(ctx.start, &root)?;
    if here.cwd != here.root {
        let sources = Sources::new(&root, ctx.no_cache)?;
        if let Ok(resolution) =
            crate::resolve::workspace(&config, &root, false, &sources, None, false)
        {
            if let Some((slot, _)) = here.enclosing(&resolution) {
                // A slot held at its pin has no branch of the topic.
                let Some(branch) = &slot.branch else {
                    return no_topic(&format!(
                        "{} is held at its pin, not on {}",
                        slot.directory, topic
                    ));
                };
                writeln!(out, "{}", branch)?;
                return Ok(());
            }
        }
    }
    writeln!(out, "{}", topic)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// join / leave
// ---------------------------------------------------------------------------

/// What `join_or_leave` does with the checkouts it is given.
#[derive(Clone, Copy, PartialEq, Eq)]
enum How {
    Join,
    /// Join the dependants of the checkouts given, or of the topic's changes.
    Dependants,
    Leave,
}

fn join_or_leave(
    ctx: &Ctx,
    dirs: &[String],
    how: How,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let leave = how == How::Leave;
    let verb = if leave { "leave" } else { "join" };
    let (config, config_root) = crate::config::load_workspace(ctx.start)?;
    let here = Here::new(ctx.start, &config_root)?;
    let sources = Sources::new(&config_root, ctx.no_cache)?;
    if dirs.is_empty() && how != How::Dependants {
        // As `git add` with nothing named: the hint, when there is one to
        // give, names the checkout the current directory is in.
        let hint = crate::resolve::workspace(&config, &config_root, false, &sources, None, false)
            .ok()
            .and_then(|r| here.enclosing(&r).map(|(_, up)| up));
        let mut message = "no checkout named".to_string();
        if let Some(up) = hint {
            message.push_str(&format!(
                "\n{}",
                crate::output::hint(
                    &format!("inside a checkout, use: git topic {} {}", verb, up),
                    crate::output::stderr()
                )
            ));
        }
        bail!("{}", message);
    }
    let Some(stores) = &sources.stores else {
        bail!(
            "git topic {} works on a developer machine: CI checkouts are copies of exact commits",
            verb
        );
    };
    stores.tidy(&config_root);
    let branch = match crate::topic::root(&config, &config_root, true) {
        Root::Topic(branch) => branch,
        Root::Pinned(branch) => bail!(
            "the root is on {}, which it pins: its checkouts stay at their pins. Start a topic \
             first: git topic start <name>",
            branch
        ),
        Root::Detached => {
            bail!("the root is on no branch. Start a topic first: git topic start <name>")
        }
    };
    let artefacts = Artefacts::new(&config, &config_root, sources.images());
    let resolution = crate::resolve::workspace(
        &config,
        &config_root,
        true,
        &sources,
        Some(&artefacts),
        ctx.verbose,
    )?;
    let placer = Placer {
        config_root: &config_root,
        sources: &sources,
        artefacts: &artefacts,
        verbose: ctx.verbose,
        fetch: true,
    };
    let targets: Vec<(String, Result<&Slot>)> = if how == How::Dependants {
        let found = dependants(
            &config_root,
            stores,
            &artefacts,
            &resolution,
            &here,
            dirs,
            out,
        )?;
        if found.is_empty() {
            writeln!(out, "nothing to join")?;
        }
        found
            .into_iter()
            .map(|slot| (slot.directory.clone(), Ok(slot)))
            .collect()
    } else {
        dirs.iter()
            .map(|dir| (dir.clone(), here.slot(&resolution, dir)))
            .collect()
    };
    let color = crate::output::stderr();
    let mut failed = 0;
    for (dir, slot) in targets {
        let done = slot.and_then(|slot| {
            if leave {
                leave_one(&config_root, &resolution, &placer, slot)
            } else {
                join_one(&config, &config_root, &placer, &branch, slot)
            }
        });
        match done {
            Ok(message) => writeln!(out, "{}", message)?,
            Err(e) => {
                failed += 1;
                writeln!(
                    err,
                    "{}",
                    crate::output::result_line(
                        crate::output::Outcome::Fail,
                        &format!("{}: {:#}", dir, e),
                        color
                    )
                )?;
            }
        }
    }
    if failed > 0 {
        bail!(
            "{} not {}",
            crate::cache::plural(failed, "checkout", "checkouts"),
            if leave {
                "taken off the topic"
            } else {
                "joined to the topic"
            }
        );
    }
    Ok(())
}

/// The checkouts `--dependants` joins, never the root. With `dirs`, the
/// dependants of each that ask for less than its newest release: a raise
/// begins so. Without, one level up from the topic's changes: the dependants
/// of each checkout carrying a change while none of them is on the topic —
/// once one is, that level is done, and a dependant left off stays off.
fn dependants<'a>(
    config_root: &Path,
    stores: &crate::store::Stores,
    artefacts: &Artefacts,
    resolution: &'a Resolution,
    here: &Here,
    dirs: &[String],
    out: &mut dyn Write,
) -> Result<Vec<&'a Slot>> {
    let requesters = |slot: &Slot, wanted: &dyn Fn(&str) -> bool| -> Vec<&'a Slot> {
        slot.requests
            .iter()
            .filter(|r| r.from != "root" && wanted(&r.revision))
            .filter_map(|r| resolution.slot(&r.from))
            .collect()
    };
    let mut found: Vec<&Slot> = Vec::new();
    if dirs.is_empty() {
        for slot in resolution.topic_slots() {
            let store = stores.repo_path(&crate::ci::remote_url(&slot.url));
            if !carries_change(config_root, &store, resolution, slot) {
                continue;
            }
            let theirs = requesters(slot, &|_| true);
            if theirs.iter().all(|r| r.topic.is_none()) {
                found.extend(theirs);
            }
        }
    } else {
        for dir in dirs {
            let slot = here.slot(resolution, dir)?;
            match crate::promote::target(stores, artefacts, slot, false)?.release(&slot.directory) {
                Ok((_, newest)) => found.extend(
                    requesters(slot, &|revision| {
                        crate::promote::below(revision, newest) == Some(true)
                    })
                    .into_iter()
                    .filter(|r| r.topic.is_none()),
                ),
                Err(why) => writeln!(out, "{}", why)?,
            }
        }
    }
    let mut seen = BTreeSet::new();
    found.retain(|s| seen.insert(s.directory.clone()));
    Ok(found)
}

/// Whether a checkout on the topic carries a change: commits on its branch
/// that its pin does not have, or uncommitted work.
fn carries_change(config_root: &Path, store: &Path, resolution: &Resolution, slot: &Slot) -> bool {
    if ahead(store, slot) > 0 {
        return true;
    }
    let dest = config_root.join(&slot.directory);
    crate::git::is_checkout(&dest)
        && crate::git::uncommitted(&dest, &resolution.planted_in(&slot.directory)).is_some()
}

/// Commits on a topic slot's branch that its pin does not have.
fn ahead(store: &Path, slot: &Slot) -> usize {
    match (&slot.topic, slot.pin()) {
        (Some(on), Some(pin)) => count(
            store,
            &["rev-list", &on.commit, &format!("^{}", pin.commit)],
        ),
        _ => 0,
    }
}

/// Put one checkout on the topic.
fn join_one(
    config: &GitScaleConfig,
    config_root: &Path,
    placer: &Placer,
    topic: &str,
    slot: &Slot,
) -> Result<String> {
    let name = slot.directory.as_str();
    if config
        .repos
        .iter()
        .any(|e| e.directory == slot.directory && e.is_override)
    {
        bail!(
            "{} is held by override in {}; remove the override first",
            name,
            CONFIG_FILENAME
        );
    }
    if let Some(by) = &slot.pinned_by {
        bail!(
            "{} pins {} for its dependencies, so {} stays at its pin",
            by,
            topic,
            name
        );
    }
    let branch = slot
        .branch
        .clone()
        .ok_or_else(|| anyhow::anyhow!("{} cannot be on topic {}", name, topic))?;
    if let Some(on) = &slot.topic {
        if on.developed {
            return Ok(format!("{} is already on {}", name, branch));
        }
    }
    let dest = config_root.join(name);
    let entry = slot.entry();
    // Joining an artefact means its source: the image goes, a worktree of
    // the same commit comes.
    let installed = if crate::git::is_checkout(&dest) {
        None
    } else {
        crate::artefact::installed(config_root, name).map(|m| m.commit)
    };
    let from = match &slot.topic {
        Some(on) => on.commit.clone(),
        None => installed
            .clone()
            .or_else(|| {
                crate::git::is_checkout(&dest)
                    .then(|| crate::git::resolve_ref(&dest, "HEAD"))
                    .flatten()
            })
            .or_else(|| slot.commit.clone())
            .ok_or_else(|| anyhow::anyhow!("{} has no commit to start from", name))?,
    };
    let stores = placer
        .sources
        .stores
        .as_ref()
        .expect("join runs with stores");
    let store = stores.update(&crate::git::remote_url(&entry))?;
    if installed.is_some() {
        crate::artefact::uninstall(config_root, name)?;
    }
    if !crate::git::is_checkout(&dest) {
        crate::store::add_worktree(&store, &dest, &from)?;
    } else if crate::store::identify(&store, &dest) == crate::store::Worktree::Foreign {
        bail!("not a gitscale worktree; move your changes out, delete it and run git scale sync");
    }
    match &slot.topic {
        // Its remote has the branch already: the store's own copy of it,
        // tracking the remote's, is where the work goes on.
        Some(_) => {
            crate::git::restore_writable(&dest)?;
            crate::git::move_checkout(
                &dest,
                &["-b", &branch, "--track", &format!("origin/{}", branch)],
            )
            .with_context(|| format!("cannot put {} on {}", name, branch))?;
        }
        None => crate::checkout::start_branch(&dest, &branch)?,
    }
    let pin = slot
        .pin()
        .map(|p| format!("{} ({})", p.revision, crate::git::short_sha(&p.commit)))
        .unwrap_or_else(|| crate::git::short_sha(&from).to_string());
    Ok(format!("{} on {}, from {}", name, branch, pin))
}

/// Take one checkout off the topic: back at its pin, its branch deleted.
fn leave_one(
    config_root: &Path,
    resolution: &Resolution,
    placer: &Placer,
    slot: &Slot,
) -> Result<String> {
    let name = slot.directory.as_str();
    let Some(on) = &slot.topic else {
        bail!("{} is not on the topic", name);
    };
    let dest = config_root.join(name);
    if remote_has_branch(&slot.url, &on.branch) {
        bail!(
            "{} exists on {}: placement follows it as long as it does. Delete it there first, \
             then run git topic leave {} again",
            on.branch,
            slot.url,
            name
        );
    }
    let (revision, pin) = match slot.pin() {
        Some(pin) => (pin.revision, pin.commit),
        // Nobody gives it a revision: the default branch is its pin.
        None => (
            String::new(),
            crate::git::resolve_ref(&dest, "refs/remotes/origin/HEAD")
                .ok_or_else(|| anyhow::anyhow!("{} has no pin to go back to", name))?,
        ),
    };
    crate::checkout::stop_branch(&dest, &on.branch, &pin, &resolution.planted_in(name), false)
        .with_context(|| format!("{} stays on {}", name, on.branch))?;
    // Back to the form it is preferred in — for an artefact, its image.
    let placed = placer.place(&slot.off_topic(&revision, &pin))?;
    Ok(format!("{} left {}: {}", name, on.branch, placed))
}

/// Whether the remote at `url` has a branch `branch`.
pub fn remote_has_branch(url: &str, branch: &str) -> bool {
    let url = crate::ci::remote_url(url);
    matches!(
        crate::git::ls_remote_revision(&url, &format!("refs/heads/{}", branch), false),
        Ok(Some(_))
    )
}

// ---------------------------------------------------------------------------
// start / switch
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn start_topic(
    ctx: &Ctx,
    name: &str,
    from: Option<&str>,
    worktree: Option<bool>,
    dir: Option<&Path>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    refuse_in_ci("start")?;
    let repo = Repo::find(ctx)?;
    let prefix = repo.prefix(true)?;
    let branch = crate::topic::with_prefix(name, prefix.as_deref());
    repo.ensure_fetch_refspec(out)?;
    repo.fetch()?;
    if repo.has_ref(&format!("refs/heads/{}", branch))
        || repo.has_ref(&format!("refs/remotes/origin/{}", branch))
    {
        bail!("{} exists: git topic switch {}", name, name);
    }
    if repo.is_pinned(&branch) {
        bail!("{} is pinned, not a topic", name);
    }
    let base = match from {
        Some(from) => {
            if repo.has_ref(&format!("refs/heads/{}", from)) {
                from.to_string()
            } else if repo.has_ref(&format!("refs/remotes/origin/{}", from)) {
                format!("origin/{}", from)
            } else {
                bail!("{} does not exist", from);
            }
        }
        None => {
            let default = repo.default_branch().ok_or_else(|| {
                anyhow::anyhow!("the root's remote names no default branch: give --from BRANCH")
            })?;
            format!("origin/{}", default)
        }
    };
    // Started from a topic: the children on it come along.
    let carry_from = from
        .map(|f| f.trim_start_matches("origin/").to_string())
        .filter(|f| !repo.is_pinned(f));
    let current = repo
        .workspace
        .as_ref()
        .and_then(|(_, root)| crate::git::current_branch(root));

    if repo.use_worktrees(worktree) {
        let cwd = ctx.cwd()?;
        let dest = match dir {
            Some(dir) => crate::paths::lexical(&cwd.join(dir)),
            None => repo.worktree_dir(&branch, prefix.as_deref()),
        };
        if dest.exists() {
            bail!(
                "{} exists; choose another with --dir",
                relative_to(&cwd, &dest)
            );
        }
        let dest_str = dest.to_string_lossy().to_string();
        repo.git(&[
            "worktree",
            "add",
            "--quiet",
            "--no-track",
            "-b",
            &branch,
            &dest_str,
            &base,
        ])
        .with_context(|| format!("cannot add a worktree for {}", branch))?;
        if let Some(from) = &carry_from {
            carry(&dest, from, &branch, ctx.no_cache);
        }
        writeln!(out, "created {}", relative_to(&cwd, &dest))?;
        if dest.join(CONFIG_FILENAME).is_file() {
            ctx.place(&dest, out, err)?;
        }
        writeln!(out, "cd {}", relative_to(&cwd, &dest))?;
        return Ok(());
    }

    let (_, root) = repo.workspace()?;
    repo.git(&["switch", "--quiet", "--no-track", "-c", &branch, &base])
        .with_context(|| format!("cannot start {}", branch))?;
    // From the branch the root was on, placement carries the children
    // itself, uncommitted edits included.
    if let Some(from) = carry_from.filter(|f| current.as_deref() != Some(f.as_str())) {
        carry(root, &from, &branch, ctx.no_cache);
    }
    writeln!(out, "started {} from {}", branch, base)?;
    ctx.place(root, out, err)
}

/// Make the remote's `branch` the upstream of the local one in the worktree
/// at `dir`, when it has none: a bare clone's branches have no upstream,
/// and `git pull` would then not know what to merge.
fn track(dir: &Path, branch: &str) {
    let tracking = format!("origin/{}", branch);
    if crate::git::ref_exists(dir, "@{upstream}")
        || !crate::git::ref_exists(dir, &format!("refs/remotes/{}", tracking))
    {
        return;
    }
    let _ = crate::git::run_git(
        &["branch", "--quiet", "--set-upstream-to", &tracking, branch],
        Some(dir),
        false,
    );
}

/// The stores' branches of topic `from` copied to `to`, for the worktree at
/// `root` to place its children on.
fn carry(root: &Path, from: &str, to: &str, no_cache: bool) {
    if let Ok(sources) = Sources::new(root, no_cache) {
        crate::checkout::carry_branches(&sources, from, to);
    }
}

fn switch(
    ctx: &Ctx,
    name: &str,
    worktree: Option<bool>,
    dir: Option<&Path>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    refuse_in_ci("switch")?;
    let repo = Repo::find(ctx)?;
    let prefix = repo.prefix(false)?;
    repo.ensure_fetch_refspec(out)?;
    repo.fetch()?;
    let mut candidates = vec![name.to_string()];
    let prefixed = crate::topic::with_prefix(name, prefix.as_deref());
    if prefixed != name {
        candidates.push(prefixed);
    }
    let local = |b: &str| repo.has_ref(&format!("refs/heads/{}", b));
    let remote = |b: &str| repo.has_ref(&format!("refs/remotes/origin/{}", b));
    let Some(branch) = candidates.into_iter().find(|b| local(b) || remote(b)) else {
        bail!("{} does not exist: git topic start {}", name, name);
    };
    let cwd = ctx.cwd()?;
    let current_root = repo.workspace.as_ref().map(|(_, root)| root.clone());

    // Already checked out somewhere: go there.
    if let Some(at) = repo.worktree_on(&branch) {
        if current_root.as_deref().and_then(|r| r.canonicalize().ok()) == Some(at.clone()) {
            writeln!(out, "already on {}", branch)?;
            return ctx.place(&at, out, err);
        }
        writeln!(out, "cd {}", relative_to(&cwd, &at))?;
        return Ok(());
    }

    if repo.use_worktrees(worktree) {
        let dest = match dir {
            Some(dir) => crate::paths::lexical(&cwd.join(dir)),
            None => repo.worktree_dir(&branch, prefix.as_deref()),
        };
        if dest.exists() {
            bail!(
                "{} exists; choose another with --dir",
                relative_to(&cwd, &dest)
            );
        }
        let dest_str = dest.to_string_lossy().to_string();
        let tracking = format!("origin/{}", branch);
        if local(&branch) {
            repo.git(&["worktree", "add", "--quiet", &dest_str, &branch])
        } else {
            repo.git(&[
                "worktree", "add", "--quiet", "--track", "-b", &branch, &dest_str, &tracking,
            ])
        }
        .with_context(|| format!("cannot add a worktree for {}", branch))?;
        track(&dest, &branch);
        writeln!(out, "created {}", relative_to(&cwd, &dest))?;
        if dest.join(CONFIG_FILENAME).is_file() {
            ctx.place(&dest, out, err)?;
        }
        writeln!(out, "cd {}", relative_to(&cwd, &dest))?;
        return Ok(());
    }

    let (_, root) = repo.workspace()?;
    if local(&branch) {
        repo.git(&["switch", "--quiet", &branch])
    } else {
        let tracking = format!("origin/{}", branch);
        repo.git(&["switch", "--quiet", "--track", "-c", &branch, &tracking])
    }
    .with_context(|| format!("cannot switch to {}", branch))?;
    track(root, &branch);
    writeln!(out, "switched to {}", branch)?;
    let root = root.to_path_buf();
    ctx.place(&root, out, err)
}

// ---------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------

/// One row of `git topic status`.
struct Row {
    repo: String,
    branch: String,
    ahead: usize,
    /// `None` with nothing to push: no commit beyond the pin.
    pushed: Option<bool>,
    state: String,
    label: &'static str,
}

fn count(dir: &Path, args: &[&str]) -> usize {
    crate::git::query(dir, args)
        .map(|listed| listed.lines().filter(|l| !l.is_empty()).count())
        .unwrap_or(0)
}

/// Whether merging `branch` into `base` changes nothing in the repository at
/// `repo`: the content test promotion uses, so a squash or rebase merge
/// counts. A branch with nothing `base` lacks is merged too.
fn merged(repo: &Path, base: &str, branch: &str) -> bool {
    if count(repo, &["rev-list", branch, &format!("^{}", base)]) == 0 {
        return true;
    }
    matches!(
        crate::promote::containment(repo, base, branch),
        Ok(crate::promote::Containment::Contained)
    )
}

/// Whether the topic has anything merged: commits beyond `base` that it
/// holds — after a squash or a rebase — or, once `base` has every one of the
/// branch's commits in its history, as a merge commit or a fast-forward
/// leaves it, commits of its own since the branch was created. `branch` is a
/// branch's ref, whose reflog says where it was created.
fn merged_with_work(repo: &Path, base: &str, branch: &str) -> bool {
    if count(repo, &["rev-list", branch, &format!("^{}", base)]) > 0 {
        return merged(repo, base, branch);
    }
    let created = crate::git::query(repo, &["reflog", "show", "--format=%H", branch])
        .and_then(|log| log.lines().last().map(str::to_string));
    let tip = crate::git::query(repo, &["rev-parse", branch]);
    matches!((created, tip), (Some(created), Some(tip)) if created != tip)
}

fn status(ctx: &Ctx, fetch: bool, format: &str, out: &mut dyn Write) -> Result<()> {
    let (config, root) = crate::config::load_workspace(ctx.start)?;
    let Root::Topic(topic) = crate::topic::root(&config, &root, fetch) else {
        bail!("not on a topic");
    };
    let sources = Sources::new(&root, ctx.no_cache)?;
    let Some(stores) = &sources.stores else {
        bail!("git topic status works on a developer machine, not in CI");
    };
    let repo = Repo::find(ctx)?;
    if fetch {
        repo.ensure_fetch_refspec(out)?;
        repo.fetch()?;
    }
    let artefacts = Artefacts::new(&config, &root, sources.images());
    let resolution = crate::resolve::workspace(
        &config,
        &root,
        fetch,
        &sources,
        Some(&artefacts),
        ctx.verbose,
    )?;
    let mut states: BTreeMap<String, State> = BTreeMap::new();
    let mut rows = Vec::new();
    for slot in resolution.topic_slots() {
        let Some(on) = &slot.topic else { continue };
        let store = stores.repo_path(&crate::ci::remote_url(&slot.url));
        let planted = resolution.planted_in(&slot.directory);
        let state = crate::promote::assess(&root, &store, slot, &planted, None);
        let ahead = ahead(&store, slot);
        let pushed = (ahead > 0)
            .then(|| count(&store, &["rev-list", &on.commit, "--not", "--remotes"]) == 0);
        rows.push(Row {
            repo: slot.directory.clone(),
            branch: on.branch.clone(),
            ahead,
            pushed,
            state: state.describe(),
            label: state.label(),
        });
        states.insert(slot.directory.clone(), state);
    }
    // A child with nothing to release holds nothing up either.
    let promoted: BTreeSet<String> = states
        .iter()
        .filter(|(_, s)| s.is_released() || **s == State::Unchanged)
        .map(|(d, _)| d.clone())
        .collect();
    let waiting: Vec<String> = states
        .keys()
        .filter(|d| !promoted.contains(*d))
        .cloned()
        .collect();
    let default = repo.default_branch();
    let base = default.as_ref().map(|d| format!("origin/{}", d));
    let root_ahead = base
        .as_ref()
        .map(|b| count(&root, &["rev-list", "HEAD", &format!("^{}", b)]))
        .unwrap_or(0);
    let root_pushed =
        (root_ahead > 0).then(|| count(&root, &["rev-list", "HEAD", "--not", "--remotes"]) == 0);
    let root_merged = base
        .as_ref()
        .is_some_and(|b| merged_with_work(&root, b, &format!("refs/heads/{}", topic)));
    let (root_state, root_label) = if root_merged {
        (
            format!("merged into {}", default.as_deref().unwrap_or("main")),
            "merged",
        )
    } else if waiting.is_empty() {
        ("ready to merge".to_string(), "ready")
    } else {
        (format!("waits on {}", waiting.join(", ")), "waits")
    };
    rows.insert(
        0,
        Row {
            repo: ".".to_string(),
            branch: topic.clone(),
            ahead: root_ahead,
            pushed: root_pushed,
            state: root_state,
            label: root_label,
        },
    );

    let requests = crate::promote::topic_requests(&config, &root, &resolution);
    let next = if states.is_empty() {
        Vec::new()
    } else {
        crate::promote::next_to_merge(&requests, &promoted)
    };
    // Commands to run now: a release waiting to be promoted comes before any
    // merge, which the promotion's pin bumps change.
    let mut then = Vec::new();
    if states.values().any(State::is_released) {
        then.push("git upgrade --commit");
    }
    if rows.iter().any(|r| r.pushed == Some(false)) {
        then.push("git scale push");
    }
    // Finishing waits on the checkouts still on the topic; `git upgrade`
    // or `git topic leave` takes them off it first.
    let still_on_topic = resolution.topic_slots().into_iter().any(|slot| {
        let dest = root.join(&slot.directory);
        slot.branch.is_some()
            && !dest.is_symlink()
            && crate::git::current_branch(&dest).as_deref() == slot.branch.as_deref()
    });
    if root_merged && !still_on_topic {
        then.push("git topic finish");
    }

    if format == "json" {
        let repos: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| {
                serde_json::json!({
                    "repo": r.repo,
                    "branch": r.branch,
                    "ahead": r.ahead,
                    "pushed": r.pushed,
                    "state": r.label,
                    "description": r.state,
                })
            })
            .collect();
        let data = serde_json::json!({
            "topic": topic,
            "repos": repos,
            "next_to_merge": next,
            "then": then,
        });
        writeln!(out, "{}", serde_json::to_string_pretty(&data)?)?;
        return Ok(());
    }
    writeln!(out, "topic {}", topic)?;
    writeln!(out)?;
    let table: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.repo.clone(),
                r.branch.clone(),
                r.ahead.to_string(),
                match r.pushed {
                    Some(true) => "yes",
                    Some(false) => "no",
                    None => "-",
                }
                .to_string(),
                r.state.clone(),
            ]
        })
        .collect();
    print_table(
        &["REPO", "BRANCH", "AHEAD", "PUSHED", "STATE"],
        &table,
        "  ",
        out,
    )?;
    if !next.is_empty() || !then.is_empty() {
        writeln!(out)?;
    }
    match (then.is_empty(), next.is_empty()) {
        (true, false) => writeln!(out, "next to merge: {}", next.join(", "))?,
        (false, true) => writeln!(out, "next: {}", then.join(", "))?,
        (false, false) => {
            writeln!(out, "next: {}", then.join(", "))?;
            writeln!(out, "then merge: {}", next.join(", "))?;
        }
        (true, true) => {}
    }
    if !fetch {
        let mut repos = vec![root.clone()];
        repos.extend(
            resolution
                .topic_slots()
                .into_iter()
                .map(|slot| stores.repo_path(&crate::ci::remote_url(&slot.url))),
        );
        offline_age(ctx, &repos, "git topic status", out)?;
    }
    Ok(())
}

/// Columns padded to their widest cell, the last one not padded.
fn print_table(
    headers: &[&str],
    rows: &[Vec<String>],
    indent: &str,
    out: &mut dyn Write,
) -> Result<()> {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: Vec<String>| -> String {
        let last = cells.len() - 1;
        let padded: Vec<String> = cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                if i == last {
                    c.clone()
                } else {
                    format!("{:<w$}", c, w = widths[i])
                }
            })
            .collect();
        format!("{}{}", indent, padded.join("   "))
            .trim_end()
            .to_string()
    };
    writeln!(
        out,
        "{}",
        line(headers.iter().map(|h| h.to_string()).collect())
    )?;
    for row in rows {
        writeln!(out, "{}", line(row.clone()))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

/// The children with a local branch of `topic` in their store — `topic`
/// itself, or `topic@v<major>` — as the workspace names them.
fn joined(
    sources: &Sources,
    names: &BTreeMap<PathBuf, String>,
    topic: &str,
) -> Vec<(String, PathBuf, String)> {
    let Some(stores) = &sources.stores else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for store in stores.all() {
        let listed = crate::git::query(
            &store,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
        )
        .unwrap_or_default();
        for branch in listed.lines() {
            if crate::topic::unversioned(branch) != topic {
                continue;
            }
            let name = names
                .get(&store)
                .cloned()
                .unwrap_or_else(|| store_label(&store));
            found.push((name, store.clone(), branch.to_string()));
        }
    }
    found.sort();
    found
}

fn list(ctx: &Ctx, fetch: bool, format: &str, out: &mut dyn Write) -> Result<()> {
    let repo = Repo::find(ctx)?;
    if fetch {
        repo.ensure_fetch_refspec(out)?;
        repo.fetch()?;
    }
    let cwd = ctx.cwd()?;
    // The stores belong to the root's common dir, whichever worktree asks.
    let anchor = match &repo.workspace {
        Some((_, root)) => root.clone(),
        None => repo.dir.clone(),
    };
    let sources = Sources {
        stores: Some(crate::store::Stores::open(&anchor)?),
        cache: None,
    };
    let worktrees = repo.worktrees();
    let names = store_names(&repo, &sources, None);
    let current = repo
        .workspace
        .as_ref()
        .and_then(|(_, root)| crate::git::current_branch(root));
    let mut topics: BTreeSet<String> = BTreeSet::new();
    let local = repo
        .query(&["for-each-ref", "--format=%(refname:short)", "refs/heads/"])
        .unwrap_or_default();
    for branch in local.lines().filter(|b| !b.is_empty()) {
        if !repo.is_pinned(branch) {
            topics.insert(branch.to_string());
        }
    }
    // A worktree's branch is listed even when pinned: `main` is the release
    // line's topic, and where it is checked out is worth seeing.
    for branch in worktrees.iter().filter_map(|(_, b)| b.as_ref()) {
        topics.insert(branch.clone());
    }
    let default = repo.default_branch();
    let base = default.as_ref().map(|d| format!("origin/{}", d));

    let mut rows = Vec::new();
    for topic in &topics {
        if fetch {
            if let Some(stores) = &sources.stores {
                for (_, store, _) in joined(&sources, &names, topic) {
                    let _ = stores.update_path(&store);
                }
            }
        }
        let children = joined(&sources, &names, topic);
        let worktree = worktrees
            .iter()
            .find(|(_, b)| b.as_deref() == Some(topic.as_str()))
            .map(|(p, _)| p.clone());
        let branch_ref = format!("refs/heads/{}", topic);
        let is_merged = base
            .as_ref()
            .is_some_and(|b| merged_with_work(&repo.dir, b, &branch_ref));
        let mut unpushed = count(&repo.dir, &["rev-list", &branch_ref, "--not", "--remotes"]);
        for (_, store, branch) in &children {
            unpushed += count(
                store,
                &[
                    "rev-list",
                    &format!("refs/heads/{}", branch),
                    "--not",
                    "--remotes",
                    "--tags",
                ],
            );
        }
        // Nothing on it yet: no commit beyond the default branch in the
        // root, nor beyond a release or the default branch in a child.
        let fresh = base
            .as_ref()
            .is_some_and(|b| count(&repo.dir, &["rev-list", &branch_ref, &format!("^{}", b)]) == 0)
            && children.iter().all(|(_, store, branch)| {
                count(
                    store,
                    &[
                        "rev-list",
                        &format!("refs/heads/{}", branch),
                        "--not",
                        "--tags",
                        "refs/remotes/origin/HEAD",
                    ],
                ) == 0
            });
        // A pinned branch — `main`, listed for its worktree — is the
        // release line, not a topic going anywhere: no state.
        let state = if repo.is_pinned(topic) {
            None
        } else if is_merged {
            Some("merged".to_string())
        } else if fresh {
            Some("new".to_string())
        } else if unpushed > 0 {
            Some(format!("{} not pushed", unpushed))
        } else {
            Some("pushed".to_string())
        };
        let mut joined_names: Vec<String> = children.iter().map(|(n, _, _)| n.clone()).collect();
        joined_names.dedup();
        rows.push((
            current.as_deref() == Some(topic.as_str()),
            topic.clone(),
            worktree.map(|p| relative_to(&cwd, &p)),
            joined_names,
            state,
        ));
    }

    if format == "json" {
        let data: Vec<serde_json::Value> = rows
            .iter()
            .map(|(current, topic, worktree, joined, state)| {
                serde_json::json!({
                    "topic": topic,
                    "current": current,
                    "worktree": worktree,
                    "joined": joined,
                    "state": state,
                })
            })
            .collect();
        writeln!(out, "{}", serde_json::to_string_pretty(&data)?)?;
        return Ok(());
    }
    if rows.is_empty() {
        writeln!(out, "No topics: start one with git topic start <name>")?;
        return Ok(());
    }
    let table: Vec<Vec<String>> = rows
        .iter()
        .map(|(current, topic, worktree, joined, state)| {
            vec![
                if *current { "▸" } else { " " }.to_string(),
                topic.clone(),
                worktree.clone().unwrap_or_else(|| "-".to_string()),
                if joined.is_empty() {
                    "-".to_string()
                } else {
                    joined.join(", ")
                },
                state.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    print_list(&table, out)?;
    if !fetch {
        offline_age(ctx, std::slice::from_ref(&repo.dir), "git topic list", out)?;
    }
    Ok(())
}

/// `list`'s table: the marker column has no header and sits tight against
/// the topic, as `▸ PROJ-12`.
fn print_list(rows: &[Vec<String>], out: &mut dyn Write) -> Result<()> {
    let headers = ["TOPIC", "WORKTREE", "JOINED", "STATE"];
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row[1..].iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |marker: &str, cells: &[String]| -> String {
        let last = cells.len() - 1;
        let padded: Vec<String> = cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                if i == last {
                    c.clone()
                } else {
                    format!("{:<w$}", c, w = widths[i])
                }
            })
            .collect();
        format!("{} {}", marker, padded.join("   "))
            .trim_end()
            .to_string()
    };
    let headers: Vec<String> = headers.iter().map(|h| h.to_string()).collect();
    writeln!(out, "{}", line(" ", &headers))?;
    for row in rows {
        writeln!(out, "{}", line(&row[0], &row[1..]))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// finish
// ---------------------------------------------------------------------------

fn finish(
    ctx: &Ctx,
    name: Option<&str>,
    force: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    refuse_in_ci("finish")?;
    let repo = Repo::find(ctx)?;
    let prefix = repo.prefix(false)?;
    repo.ensure_fetch_refspec(out)?;
    let current = repo
        .workspace
        .as_ref()
        .and_then(|(_, root)| crate::git::current_branch(root));
    let branch = match name {
        Some(name) => {
            let prefixed = crate::topic::with_prefix(name, prefix.as_deref());
            [name.to_string(), prefixed]
                .into_iter()
                .find(|b| repo.has_ref(&format!("refs/heads/{}", b)))
                .ok_or_else(|| anyhow::anyhow!("{} is not a topic of this repository", name))?
        }
        None => match current.clone() {
            Some(branch) if !repo.is_pinned(&branch) => branch,
            _ => bail!("not on a topic: name the one to finish, git topic finish <name>"),
        },
    };
    if repo.is_pinned(&branch) {
        bail!("{} is pinned, not a topic", branch);
    }
    repo.fetch()?;
    let default = repo
        .default_branch()
        .ok_or_else(|| anyhow::anyhow!("the root's remote names no default branch"))?;
    let base = format!("origin/{}", default);
    let branch_ref = format!("refs/heads/{}", branch);

    // 1. Merged, by content: a squash or rebase merge counts.
    if !merged(&repo.dir, &base, &branch_ref) && !force {
        bail!(
            "{} is not merged into {}; --force to drop it",
            branch,
            default
        );
    }

    // Where the topic is checked out. A plain clone's own working tree goes
    // back to the default branch; any other worktree — every one of a bare
    // repository's, or one added with --worktree — is removed.
    let worktree = repo.worktree_on(&branch);
    let main = (!repo.bare)
        .then(|| {
            repo.common
                .parent()
                .map(|p| p.canonicalize().unwrap_or(p.to_path_buf()))
        })
        .flatten();
    let remove = worktree.as_ref().filter(|w| main.as_ref() != Some(*w));
    let cwd = ctx.cwd()?;
    // Where to go once this worktree is gone: asked now, while it is there
    // to ask from.
    let back = repo
        .worktree_on(&default)
        .or(main)
        .unwrap_or_else(|| repo.common.parent().unwrap_or(&repo.common).to_path_buf());

    // How the stores are named in what this says: by the checkouts made
    // from them, asked now, while the worktree is there to ask.
    let sources = Sources {
        stores: Some(crate::store::Stores::open(&repo.common)?),
        cache: None,
    };
    let names = store_names(&repo, &sources, worktree.as_deref());

    // 2. Checkouts still on the topic: brought to it by `git upgrade` or
    // `git topic leave` — or, with --force, back to their pins. 3.
    // Uncommitted changes in whatever moves or goes are refused, --force or
    // not: nothing else holds them.
    if let Some(at) = &worktree {
        let still = children_on_topic(at, ctx.no_cache)?;
        if !still.is_empty() && !force {
            let mut lines: Vec<String> = still
                .iter()
                .map(|dir| {
                    format!(
                        "{} is still on the topic: git upgrade, or git topic leave {}",
                        dir, dir
                    )
                })
                .collect();
            lines.push("--force takes them back to their pins".to_string());
            bail!("{}", lines.join("\n"));
        }
        let work = uncommitted_work(at, &still, remove.is_some(), ctx.no_cache);
        if !work.is_empty() {
            bail!(
                "uncommitted changes in {}: commit or stash them (git scale stash -u), or \
                 discard them (git scale reset --hard && git scale clean -fd)",
                work.join(", ")
            );
        }
    }

    // 4. The topic's branches, the root's and each store's alike, all go.
    // Commits on one that no remote branch or tag has, and whose changes
    // the default branch lacks, would go with it: refused, unless --force.
    let mut branches = vec![(".".to_string(), repo.common.clone(), branch.clone())];
    branches.extend(joined(&sources, &names, &branch));
    let mut losing: Vec<(String, String, usize, String)> = Vec::new();
    for (label, repo_dir, local) in &branches {
        let reference = format!("refs/heads/{}", local);
        let unique = count(
            repo_dir,
            &["rev-list", &reference, "--not", "--remotes", "--tags"],
        );
        let into = if label == "." {
            Some(base.as_str())
        } else {
            crate::git::ref_exists(repo_dir, "refs/remotes/origin/HEAD")
                .then_some("refs/remotes/origin/HEAD")
        };
        if unique > 0 && !into.is_some_and(|b| merged(repo_dir, b, &reference)) {
            let tip = crate::git::query(repo_dir, &["rev-parse", "--short", &reference])
                .unwrap_or_default();
            losing.push((label.clone(), local.clone(), unique, tip));
        }
    }
    if !losing.is_empty() && !force {
        let mut lines: Vec<String> = losing
            .iter()
            .map(|(label, local, unique, _)| {
                format!(
                    "{} in {} has {} no remote has",
                    local,
                    label,
                    crate::cache::plural(*unique, "commit", "commits")
                )
            })
            .collect();
        lines.push("push them first, or --force to drop them".to_string());
        bail!("{}", lines.join("\n"));
    }

    // Git runs from the common dir from here on: the worktree it was asked
    // from may be the one that goes.
    match (&worktree, remove) {
        (_, Some(at)) => {
            // Its checkouts go with it: none holds work, as checked above, and
            // their branches are in the stores.
            std::fs::remove_dir_all(at)
                .with_context(|| format!("cannot remove {}", at.display()))?;
            crate::git::run_git(&["worktree", "prune"], Some(&repo.common), true)?;
            for store in sources.stores.iter().flat_map(|s| s.all()) {
                let _ = crate::git::run_git(&["worktree", "prune"], Some(&store), false);
            }
            writeln!(out, "removed {}", relative_to(&cwd, at))?;
        }
        (Some(at), None) => {
            let run = |args: &[&str]| crate::git::run_git(args, Some(at), true);
            if crate::git::ref_exists(at, &format!("refs/heads/{}", default)) {
                run(&["switch", "--quiet", &default])?;
                run(&["merge", "--ff-only", "--quiet", &base])
                    .with_context(|| format!("{} cannot fast-forward to {}", default, base))?;
            } else {
                run(&["switch", "--quiet", "--track", "-c", &default, &base])?;
            }
            writeln!(out, "switched to {}", default)?;
            ctx.place(at, out, err)?;
        }
        (None, None) => {}
    }

    // Remote branches are never touched. A dropped branch's tip is said, as
    // `git branch -D` says it, for a slip to be undone.
    let mut deleted: Vec<String> = Vec::new();
    for (label, repo_dir, local) in &branches {
        let gone = crate::git::run_git(&["branch", "--quiet", "-D", local], Some(repo_dir), false)
            .is_ok_and(|o| o.status.success());
        if gone && !losing.iter().any(|(l, lo, ..)| l == label && lo == local) {
            deleted.push(named(label, local, &branch));
        }
    }
    if !deleted.is_empty() {
        writeln!(out, "deleted {} in {}", branch, deleted.join(", "))?;
    }
    for (label, local, unique, tip) in &losing {
        writeln!(
            out,
            "dropped {} in {} (was {}): {} no remote had",
            local,
            label,
            tip,
            crate::cache::plural(*unique, "commit", "commits")
        )?;
    }

    if let Some(at) = remove {
        if cwd.starts_with(at) {
            writeln!(out, "cd {}", relative_to(&cwd, &back))?;
        }
    }
    Ok(())
}

/// Where a branch of topic `topic` is, for what `finish` says: the
/// directory, with the branch when it is a major's own, `NAME@vN`.
fn named(label: &str, local: &str, topic: &str) -> String {
    if local == topic {
        label.to_string()
    } else {
        format!("{} ({})", label, local)
    }
}

/// The stores of the root, each named by a checkout made from it — in the
/// worktree at `root`, else in the workspace asked from, else in the first
/// worktree with a `.gitscale.toml`.
fn store_names(repo: &Repo, sources: &Sources, root: Option<&Path>) -> BTreeMap<PathBuf, String> {
    let mut names = BTreeMap::new();
    let root = root
        .map(Path::to_path_buf)
        .or_else(|| repo.workspace.as_ref().map(|(_, root)| root.clone()))
        .or_else(|| {
            repo.worktrees()
                .into_iter()
                .map(|(dir, _)| dir)
                .find(|dir| dir.join(CONFIG_FILENAME).is_file())
        });
    let (Some(root), Some(stores)) = (root, &sources.stores) else {
        return names;
    };
    let Ok(config) = load_config(&root.join(CONFIG_FILENAME)) else {
        return names;
    };
    let Ok(resolution) = crate::resolve::workspace(&config, &root, false, sources, None, false)
    else {
        return names;
    };
    for slot in &resolution.slots {
        names
            .entry(stores.repo_path(&crate::ci::remote_url(&slot.url)))
            .or_insert_with(|| slot.directory.clone());
    }
    names
}

/// A store named by its repository, for one no checkout here is made from.
fn store_label(store: &Path) -> String {
    crate::git::origin_url(store)
        .map(|u| crate::resolution::repo_name(&u))
        .unwrap_or_else(|| {
            store
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
}

/// What `finish` would lose in the worktree at `root`: uncommitted changes
/// in the root, in each checkout in `moving`, and — when the worktree is to
/// be removed — in every checkout, or commits a detached one holds on no
/// branch. Named by directory.
fn uncommitted_work(root: &Path, moving: &[String], removing: bool, no_cache: bool) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(config) = load_config(&root.join(CONFIG_FILENAME)) else {
        return found;
    };
    let Ok(sources) = Sources::new(root, no_cache) else {
        return found;
    };
    let Ok(resolution) = crate::resolve::workspace(&config, root, false, &sources, None, false)
    else {
        return found;
    };
    if resolution.root_changes(root).is_some() {
        found.push(".".to_string());
    }
    for slot in &resolution.slots {
        let dest = root.join(&slot.directory);
        if !crate::git::is_checkout(&dest) || dest.is_symlink() {
            continue;
        }
        if !removing && !moving.contains(&slot.directory) {
            continue;
        }
        let planted = resolution.planted_in(&slot.directory);
        let lost = if crate::git::current_branch(&dest).is_some() {
            crate::git::uncommitted(&dest, &planted).is_some()
        } else {
            crate::git::local_work(&dest, &planted).is_some()
        };
        if lost {
            found.push(slot.directory.clone());
        }
    }
    found
}

/// The children of the worktree at `root` on their topic branch.
fn children_on_topic(root: &Path, no_cache: bool) -> Result<Vec<String>> {
    let config = load_config(&root.join(CONFIG_FILENAME))?;
    let sources = Sources::new(root, no_cache)?;
    let resolution = crate::resolve::workspace(&config, root, false, &sources, None, false)?;
    Ok(resolution
        .slots
        .iter()
        .filter(|slot| {
            let dest = root.join(&slot.directory);
            slot.branch.is_some()
                && crate::git::is_checkout(&dest)
                && !dest.is_symlink()
                && crate::git::current_branch(&dest).as_deref() == slot.branch.as_deref()
        })
        .map(|slot| slot.directory.clone())
        .collect())
}

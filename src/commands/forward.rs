//! `git scale <git command>`: run a git command across the workspace.
//!
//! It runs in the root and every checkout on the topic — `--for` narrows
//! that, `--foreach` adds the checkouts at their pins — dependencies before the
//! repositories that ask for them, the root last. What it moved is placed
//! again afterwards: `pull` always, online; `fetch` never, refreshing the
//! stores instead; any other command when it changed a `HEAD`, fetching only
//! what resolution then lacks.

use anyhow::{bail, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::output::{self, Outcome};
use crate::resolution::Resolution;
use crate::resolve::Network;
use crate::store::Sources;
use crate::Io;

/// GitScale's options for a git command: before it, or anywhere among its
/// arguments up to `--`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// `--for DIR`, repeatable.
    pub select: Vec<String>,
    /// `--foreach`.
    pub foreach: bool,
    /// `--parallel[=N]`: `Some(0)` without a number.
    pub parallel: Option<usize>,
    /// `--force-sync`.
    pub force_sync: bool,
}

impl Options {
    /// Take GitScale's options out of the git command's arguments, up to
    /// `--`, adding them to these: what is left is git's.
    pub fn take_from(&mut self, args: &[OsString]) -> Result<Vec<OsString>> {
        let mut rest: Vec<OsString> = Vec::new();
        let mut words = args.iter();
        while let Some(word) = words.next() {
            let text = word.to_string_lossy();
            match text.as_ref() {
                "--" => {
                    rest.push(word.clone());
                    rest.extend(words.cloned());
                    break;
                }
                "--foreach" => self.foreach = true,
                "--force-sync" => self.force_sync = true,
                "--parallel" => self.parallel = Some(0),
                "--for" => match words.next() {
                    Some(dir) => self.select.push(dir.to_string_lossy().into_owned()),
                    None => bail!("--for needs a directory"),
                },
                _ => {
                    if let Some(dir) = text.strip_prefix("--for=") {
                        self.select.push(dir.to_string());
                    } else if let Some(n) = text.strip_prefix("--parallel=") {
                        let n = n.parse().map_err(|_| {
                            anyhow::anyhow!("--parallel={}: give the number of repositories", n)
                        })?;
                        self.parallel = Some(n);
                    } else {
                        rest.push(word.clone());
                    }
                }
            }
        }
        Ok(rest)
    }
}

/// The commands git itself pages.
const PAGED: &[&str] = &["log", "show", "diff", "grep", "blame", "shortlog", "reflog"];

/// The commands `[forward] parallel` makes parallel by default.
const PARALLEL_BY_DEFAULT: &[&str] = &["fetch", "pull", "push"];

/// What an editor under `--parallel` says, so the failure can say why.
const NO_EDITOR: &str = "gitscale: no terminal for an editor";

/// One repository a git command runs in.
#[derive(Debug, Clone)]
struct Repo {
    /// `.` for the root, else the checkout's directory.
    name: String,
    dir: PathBuf,
}

/// What a repository's run came to.
#[derive(Debug, Clone)]
enum Ran {
    Skipped(String),
    Exited { code: Option<i32>, no_editor: bool },
}

impl Ran {
    fn failed(&self) -> bool {
        matches!(self, Ran::Exited { code, .. } if *code != Some(0))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    start: Option<&Path>,
    before: &Options,
    args: &[OsString],
    verbose: bool,
    no_cache: bool,
    io: Io,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let (config, root) = crate::config::load_workspace(start)?;
    // The git command itself comes first; GitScale's options may follow it.
    let mut opts = before.clone();
    let mut args = args.to_vec();
    if args.len() > 1 {
        let rest = opts.take_from(&args[1..])?;
        args.truncate(1);
        args.extend(rest);
    }
    let opts = &opts;
    let args = args.as_slice();
    let typed = args
        .first()
        .map(|a| a.to_string_lossy().into_owned())
        .unwrap_or_default();
    let command = alias_target(&root, &typed);
    if command == "fetch" && opts.force_sync {
        bail!("--force-sync has no effect with fetch");
    }
    let color_out = output::stdout();
    let color_err = output::stderr();

    // Offline: what the workspace is now decides where the command runs.
    let sources = Sources::new(&root, no_cache)?;
    let resolution = match crate::resolve::workspace(&config, &root, false, &sources, None, false) {
        Ok(resolution) => Some(resolution),
        Err(e) => {
            writeln!(
                err,
                "{} cannot resolve the workspace ({:#}); running in the root only",
                output::paint(color_err, output::YELLOW, "warning:"),
                e
            )?;
            None
        }
    };
    let empty = Resolution::default();
    let resolved = resolution.as_ref().unwrap_or(&empty);
    let here = crate::paths::Here::new(start, &root)?;
    let selected = select(&here, resolved, opts)?;
    let order = order(resolved, &selected);
    let total = order.len();

    let parallel = match opts.parallel {
        Some(0) => default_parallelism(),
        Some(n) => n,
        None if PARALLEL_BY_DEFAULT.contains(&command.as_str()) => {
            config.forward.parallel.unwrap_or(1)
        }
        None => 1,
    };

    // One pager for everything, when git would page this command.
    let mut pager = if io.inherit && io.streams.stdout_tty {
        pager_for(&root, &command).and_then(|cmd| Pager::spawn(&cmd).ok())
    } else {
        None
    };

    let paging = pager.is_some();
    let before: Vec<Head> = order.iter().map(|r| Head::of(&r.dir)).collect();
    // Git writes into the pager straight, through its own copy of the pipe.
    let pager_fd = match &pager {
        Some(p) => {
            use std::os::fd::AsFd;
            Some(p.stdin.as_fd().try_clone_to_owned()?)
        }
        None => None,
    };
    let results = {
        let w: &mut dyn Write = match pager.as_mut() {
            Some(p) => &mut p.stdin,
            None => out,
        };
        let ctx = RunCtx {
            args,
            command: &command,
            color: color_out,
            io,
            pager: pager_fd.as_ref(),
        };
        if parallel > 1 {
            run_parallel(
                &ctx,
                &order,
                &deps(resolved, &order),
                parallel,
                paging,
                w,
                err,
            )
        } else {
            run_sequential(&ctx, &order, w, err)
        }
    };
    // Quitting the pager early is how one stops reading, as with git's own.
    if let Some(p) = pager.as_mut() {
        if p.child.try_wait().ok().flatten().is_some() || results.as_ref().is_err_and(broken_pipe) {
            drop(pager_fd);
            if let Some(pager) = pager {
                pager.finish();
            }
            return Ok(());
        }
    }
    let results = results?;

    // Changes in a checkout the command did not run in: most likely made
    // there by hand, for a topic it is not on.
    if let Some(resolution) = &resolution {
        let w: &mut dyn Write = err;
        for slot in &resolution.slots {
            let dest = root.join(&slot.directory);
            if order.iter().any(|r| r.name == slot.directory)
                || !crate::git::is_checkout(&dest)
                || dest.is_symlink()
                || on_topic(&root, slot)
            {
                continue;
            }
            if crate::git::uncommitted(&dest, &resolution.planted_in(&slot.directory)).is_some() {
                writeln!(
                    w,
                    "{}",
                    output::paint(
                        color_err,
                        output::YELLOW,
                        &format!(
                            "{} has changes but is not on the topic: git topic join {}",
                            slot.directory, slot.directory
                        )
                    )
                )?;
            }
        }
    }

    // Then what the command moved is placed again.
    let moved = order
        .iter()
        .zip(&before)
        .any(|(repo, head)| Head::of(&repo.dir) != *head);
    let after = match command.as_str() {
        "pull" => Some(Network::Online),
        "fetch" => None,
        _ if moved => Some(Network::OnMiss),
        _ => None,
    };
    let placed = {
        let w: &mut dyn Write = match pager.as_mut() {
            Some(p) => &mut p.stdin,
            None => out,
        };
        if command == "fetch" {
            writeln!(
                w,
                "{}",
                output::header(None, "refreshing", None, output::header_width(), color_out)
            )?;
            w.flush()?;
            crate::commands::fetch::refresh(
                &config,
                &root,
                verbose,
                no_cache,
                io.interactive && !paging,
                w,
                err,
            )
        } else if let Some(network) = after {
            writeln!(
                w,
                "{}",
                output::header(None, "placing", None, output::header_width(), color_out)
            )?;
            w.flush()?;
            // Reread: the command may have changed the root's own config.
            let config = crate::config::load_config(&root.join(crate::config::CONFIG_FILENAME))
                .unwrap_or(config.clone());
            crate::commands::sync::place(
                &config,
                &root,
                &crate::commands::sync::Placement {
                    dirs: &[],
                    network,
                    force: opts.force_sync,
                    leave: None,
                    heading: "",
                },
                verbose,
                no_cache,
                io.interactive && !paging,
                w,
                err,
            )
        } else {
            Ok(())
        }
    };
    drop(pager_fd);
    if let Some(pager) = pager {
        pager.finish();
    }

    let failed: Vec<&str> = order
        .iter()
        .zip(&results)
        .filter(|(_, ran)| ran.failed())
        .map(|(repo, _)| repo.name.as_str())
        .collect();
    if failed.is_empty() && placed.is_ok() {
        return Ok(());
    }
    if !failed.is_empty() {
        writeln!(
            err,
            "{}",
            output::paint(
                color_err,
                output::BOLD_RED,
                &format!(
                    "{} of {} {} failed: {}",
                    failed.len(),
                    total,
                    if total == 1 {
                        "repository"
                    } else {
                        "repositories"
                    },
                    failed.join(", ")
                )
            )
        )?;
    }
    if let Err(e) = placed {
        writeln!(
            err,
            "{}",
            output::paint(
                color_err,
                output::BOLD_RED,
                &format!("placement failed: {:#}", e)
            )
        )?;
    }
    Err(crate::reported())
}

/// The command an alias stands for: the first word of `alias.<name>`, when
/// it is not a shell alias. A shell alias keeps its own name.
fn alias_target(root: &Path, name: &str) -> String {
    match crate::git::query(root, &["config", "--get", &format!("alias.{}", name)]) {
        Some(value) if !value.trim_start().starts_with('!') => {
            value.split_whitespace().next().unwrap_or(name).to_string()
        }
        _ => name.to_string(),
    }
}

/// The number of repositories `--parallel` runs at once without a number.
fn default_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(16)
}

/// Whether the checkout of `slot` is on the topic: on the branch resolution
/// gives its slot.
fn on_topic(root: &Path, slot: &crate::resolution::Slot) -> bool {
    let dest = root.join(&slot.directory);
    slot.branch.is_some()
        && crate::git::is_checkout(&dest)
        && !dest.is_symlink()
        && crate::git::current_branch(&dest).as_deref() == slot.branch.as_deref()
}

/// The repositories the command runs in: the root and the checkouts on the
/// topic by default, every checkout with `--foreach`, exactly the named ones
/// with `--for`. Links and entries with no checkout are never among them.
fn select(here: &crate::paths::Here, resolution: &Resolution, opts: &Options) -> Result<Vec<Repo>> {
    let root = Repo {
        name: ".".to_string(),
        dir: here.root.clone(),
    };
    let child = |dir: &str| Repo {
        name: dir.to_string(),
        dir: here.root.join(dir),
    };
    let has_checkout = |dir: &str| {
        let dest = here.root.join(dir);
        crate::git::is_checkout(&dest) && !dest.is_symlink()
    };
    if !opts.select.is_empty() {
        let mut chosen: Vec<Repo> = Vec::new();
        for arg in &opts.select {
            let repo = match here.name(resolution, arg, true)? {
                crate::paths::Named::Root => root.clone(),
                crate::paths::Named::Slot(dir) => {
                    if !has_checkout(&dir) {
                        bail!("{} has no checkout of its own here", dir);
                    }
                    let slot = resolution.slot(&dir).expect("named by the resolution");
                    if !opts.foreach && !on_topic(&here.root, slot) {
                        bail!("{} is not on the topic: git topic join {}", dir, dir);
                    }
                    child(&dir)
                }
            };
            if !chosen.iter().any(|r| r.name == repo.name) {
                chosen.push(repo);
            }
        }
        return Ok(chosen);
    }
    let mut chosen = vec![root];
    for slot in &resolution.slots {
        if has_checkout(&slot.directory) && (opts.foreach || on_topic(&here.root, slot)) {
            chosen.push(child(&slot.directory));
        }
    }
    Ok(chosen)
}

/// Which slots each slot asks for, by directory: from the requests
/// resolution recorded.
fn requests(resolution: &Resolution) -> BTreeMap<String, BTreeSet<String>> {
    let mut asks: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for slot in &resolution.slots {
        asks.entry(slot.directory.clone()).or_default();
        for request in &slot.requests {
            if request.from != "root" {
                asks.entry(request.from.clone())
                    .or_default()
                    .insert(slot.directory.clone());
            }
        }
    }
    asks
}

/// `selected` in running order: a dependency before the repositories that
/// ask for it, by path where nothing orders two, the root last.
fn order(resolution: &Resolution, selected: &[Repo]) -> Vec<Repo> {
    let asks = requests(resolution);
    // Kahn's, taking the first path that is ready each time.
    let mut waiting: BTreeMap<&str, usize> =
        asks.keys().map(|d| (d.as_str(), asks[d].len())).collect();
    let mut asked_by: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (requester, wanted) in &asks {
        for dep in wanted {
            asked_by
                .entry(dep.as_str())
                .or_default()
                .push(requester.as_str());
        }
    }
    let mut ready: BTreeSet<&str> = waiting
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(d, _)| *d)
        .collect();
    let mut sorted: Vec<&str> = Vec::new();
    while let Some(next) = ready.iter().next().copied() {
        ready.remove(next);
        sorted.push(next);
        for requester in asked_by.get(next).cloned().unwrap_or_default() {
            let n = waiting
                .get_mut(requester)
                .expect("every requester is a slot");
            *n -= 1;
            if *n == 0 {
                ready.insert(requester);
            }
        }
    }
    // A cycle cannot resolve, but should one get here: by path.
    for dir in asks.keys() {
        if !sorted.contains(&dir.as_str()) {
            sorted.push(dir);
        }
    }
    let mut ordered: Vec<Repo> = sorted
        .iter()
        .filter_map(|dir| selected.iter().find(|r| r.name == *dir).cloned())
        .collect();
    // Anything resolution does not know, by path, then the root.
    let mut rest: Vec<Repo> = selected
        .iter()
        .filter(|r| r.name != "." && !ordered.iter().any(|o| o.name == r.name))
        .cloned()
        .collect();
    rest.sort_by(|a, b| a.name.cmp(&b.name));
    ordered.extend(rest);
    if let Some(root) = selected.iter().find(|r| r.name == ".") {
        ordered.push(root.clone());
    }
    ordered
}

/// For each repository of `order`, the earlier ones it waits for under
/// `--parallel`: every selected repository it depends on, however
/// indirectly. The root waits for every other.
fn deps(resolution: &Resolution, order: &[Repo]) -> Vec<Vec<usize>> {
    let asks = requests(resolution);
    let index: HashMap<&str, usize> = order
        .iter()
        .enumerate()
        .map(|(i, r)| (r.name.as_str(), i))
        .collect();
    order
        .iter()
        .map(|repo| {
            if repo.name == "." {
                return (0..order.len()).filter(|i| order[*i].name != ".").collect();
            }
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            let mut stack: Vec<&str> = asks
                .get(&repo.name)
                .map(|s| s.iter().map(String::as_str).collect())
                .unwrap_or_default();
            while let Some(dep) = stack.pop() {
                if seen.insert(dep) {
                    if let Some(more) = asks.get(dep) {
                        stack.extend(more.iter().map(String::as_str));
                    }
                }
            }
            seen.iter().filter_map(|d| index.get(d).copied()).collect()
        })
        .collect()
}

/// Where a repository's `HEAD` is: its commit and the branch it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Head(Option<String>, Option<String>);

impl Head {
    fn of(dir: &Path) -> Head {
        Head(
            crate::git::resolve_ref(dir, "HEAD"),
            crate::git::current_branch(dir),
        )
    }
}

/// What every run of one command shares.
struct RunCtx<'a> {
    args: &'a [OsString],
    command: &'a str,
    /// GitScale colours stdout: git is told to as well.
    color: bool,
    io: Io,
    /// The pager's stdin, which git's output goes to too.
    pager: Option<&'a std::os::fd::OwnedFd>,
}

impl RunCtx<'_> {
    /// `git --no-pager -c push.autoSetupRemote=true [-c color.ui=always] …`
    /// in `dir`, with gitscale's hook held off: what the command moved is
    /// placed afterwards, once.
    fn git(&self, dir: &Path, args: &[OsString]) -> Command {
        let mut cmd = crate::git::git_command();
        if let Some(auth) = crate::ci::active() {
            cmd.args(auth.git_config_args());
        }
        cmd.args(["--no-pager", "-c", "push.autoSetupRemote=true"]);
        if self.color {
            cmd.args(["-c", "color.ui=always"]);
        }
        cmd.args(args).current_dir(dir).env("GITSCALE_HOOK", "1");
        cmd
    }

    /// Why `repo` is skipped, when it is: `commit` with nothing to commit,
    /// as `--dry-run` answers. `--amend` and `--allow-empty` commit with
    /// nothing staged, which `--dry-run` does not know: never skipped.
    fn skip(&self, repo: &Repo) -> Option<String> {
        if self.command != "commit"
            || self.args[1..]
                .iter()
                .any(|a| a == "--amend" || a == "--allow-empty")
        {
            return None;
        }
        let mut dry: Vec<OsString> = vec![self.args[0].clone(), "--dry-run".into()];
        dry.extend(self.args[1..].iter().cloned());
        let status = self
            .git(&repo.dir, &dry)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()?;
        (status.code() == Some(1)).then(|| "nothing to commit".to_string())
    }
}

fn header_line(ctx: &RunCtx, n: usize, total: usize, repo: &Repo, skip: Option<&str>) -> String {
    output::header(
        Some((n, total)),
        &repo.name,
        skip,
        output::header_width(),
        ctx.color,
    )
}

fn run_sequential(
    ctx: &RunCtx,
    order: &[Repo],
    w: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<Vec<Ran>> {
    let total = order.len();
    let mut results = Vec::new();
    for (i, repo) in order.iter().enumerate() {
        if let Some(reason) = ctx.skip(repo) {
            writeln!(w, "{}", header_line(ctx, i + 1, total, repo, Some(&reason)))?;
            results.push(Ran::Skipped(reason));
            continue;
        }
        writeln!(w, "{}", header_line(ctx, i + 1, total, repo, None))?;
        w.flush()?;
        let mut cmd = ctx.git(&repo.dir, ctx.args);
        let ran = if ctx.io.inherit {
            // The terminal is git's: an editor, `add -p` and a credential
            // prompt all need it.
            if let Some(pager) = ctx.pager {
                cmd.stdout(Stdio::from(pager.try_clone()?));
                if ctx.io.streams.stderr_tty {
                    cmd.stderr(Stdio::from(pager.try_clone()?));
                }
            }
            match cmd.status() {
                Ok(status) => Ran::Exited {
                    code: status.code(),
                    no_editor: false,
                },
                Err(e) => {
                    writeln!(err, "cannot run git: {}", e)?;
                    Ran::Exited {
                        code: None,
                        no_editor: false,
                    }
                }
            }
        } else {
            // Output collected: nobody can answer an editor or a prompt.
            let output = cmd
                .stdin(Stdio::null())
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_EDITOR", editor_refusal())
                .env("GIT_SEQUENCE_EDITOR", editor_refusal())
                .output()?;
            w.write_all(&output.stdout)?;
            err.write_all(&output.stderr)?;
            Ran::Exited {
                code: output.status.code(),
                no_editor: false,
            }
        };
        results.push(ran);
    }
    Ok(results)
}

/// One finished run under `--parallel`.
struct Done {
    index: usize,
    ran: Ran,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[allow(clippy::too_many_arguments)]
fn run_parallel(
    ctx: &RunCtx,
    order: &[Repo],
    deps: &[Vec<usize>],
    limit: usize,
    paging: bool,
    w: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<Vec<Ran>> {
    use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
    use std::sync::{Condvar, Mutex};

    let total = order.len();
    // Skipped repositories are known before anything runs.
    let skips: Vec<Option<String>> = order.iter().map(|r| ctx.skip(r)).collect();
    let live = ctx.io.inherit && ctx.io.streams.stderr_tty && !paging;
    let color_err = output::stderr();
    let mp = live.then(MultiProgress::new);
    let bars: Vec<Option<ProgressBar>> = order
        .iter()
        .map(|repo| {
            mp.as_ref().map(|mp| {
                let pb = mp.add(ProgressBar::new_spinner());
                pb.set_style(
                    ProgressStyle::with_template("  {spinner:.cyan} {msg}")
                        .unwrap()
                        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
                );
                pb.set_message(format!("{:<24} waiting", repo.name));
                pb.enable_steady_tick(std::time::Duration::from_millis(80));
                pb
            })
        })
        .collect();

    struct Sched {
        started: Vec<bool>,
        done: Vec<bool>,
    }
    let sched = Mutex::new(Sched {
        started: skips.iter().map(Option::is_some).collect(),
        done: skips.iter().map(Option::is_some).collect(),
    });
    let wake = Condvar::new();
    let (tx, rx) = std::sync::mpsc::channel::<Done>();
    let style = output::current();

    let mut finished: Vec<Option<Done>> = (0..total).map(|_| None).collect();
    for (i, skip) in skips.iter().enumerate() {
        if let Some(reason) = skip {
            finished[i] = Some(Done {
                index: i,
                ran: Ran::Skipped(reason.clone()),
                stdout: Vec::new(),
                stderr: Vec::new(),
            });
            if let Some(pb) = &bars[i] {
                pb.set_style(ProgressStyle::with_template("{msg}").unwrap());
                pb.finish_with_message(output::result_line(
                    Outcome::Skip,
                    &format!("{} ({})", order[i].name, reason),
                    color_err,
                ));
            }
        }
    }
    let mut printed = 0usize;
    let mut print_ready =
        |finished: &mut Vec<Option<Done>>, w: &mut dyn Write, err: &mut dyn Write| -> Result<()> {
            while printed < total {
                let Some(done) = finished[printed].as_ref() else {
                    break;
                };
                let repo = &order[printed];
                let mut block = Vec::new();
                let mut errs = Vec::new();
                match &done.ran {
                    Ran::Skipped(reason) => {
                        writeln!(
                            block,
                            "{}",
                            header_line(ctx, printed + 1, total, repo, Some(reason))
                        )?;
                    }
                    Ran::Exited { code, no_editor } => {
                        writeln!(
                            block,
                            "{}",
                            header_line(ctx, printed + 1, total, repo, None)
                        )?;
                        block.extend_from_slice(&done.stdout);
                        errs.extend_from_slice(&done.stderr);
                        if *code != Some(0) && !live {
                            let why = if *no_editor {
                                " (--parallel: no terminal; run without it)"
                            } else {
                                ""
                            };
                            writeln!(
                                errs,
                                "{}",
                                output::result_line(
                                    Outcome::Fail,
                                    &format!("{}: exit {}{}", repo.name, code.unwrap_or(-1), why),
                                    color_err
                                )
                            )?;
                        }
                    }
                }
                let mut write = || -> std::io::Result<()> {
                    w.write_all(&block)?;
                    w.flush()?;
                    err.write_all(&errs)?;
                    err.flush()
                };
                match &mp {
                    Some(mp) => mp.suspend(write)?,
                    None => write()?,
                }
                printed += 1;
            }
            Ok(())
        };
    print_ready(&mut finished, w, err)?;

    std::thread::scope(|scope| -> Result<()> {
        for _ in 0..limit.min(total.max(1)) {
            let tx = tx.clone();
            let sched = &sched;
            let wake = &wake;
            let bars = &bars;
            scope.spawn(move || {
                output::set(style);
                loop {
                    let next = {
                        let mut s = sched.lock().unwrap();
                        loop {
                            if s.started.iter().all(|x| *x) {
                                break None;
                            }
                            let ready = (0..total)
                                .find(|i| !s.started[*i] && deps[*i].iter().all(|d| s.done[*d]));
                            match ready {
                                Some(i) => {
                                    s.started[i] = true;
                                    break Some(i);
                                }
                                None => s = wake.wait(s).unwrap(),
                            }
                        }
                    };
                    let Some(i) = next else { return };
                    if let Some(pb) = &bars[i] {
                        pb.set_message(format!("{:<24} running...", order[i].name));
                    }
                    let output = ctx
                        .git(&order[i].dir, ctx.args)
                        .stdin(Stdio::null())
                        .env("GIT_TERMINAL_PROMPT", "0")
                        .env("GIT_EDITOR", editor_refusal())
                        .env("GIT_SEQUENCE_EDITOR", editor_refusal())
                        .output();
                    let (ran, stdout, stderr) = match output {
                        Ok(o) => {
                            let no_editor = String::from_utf8_lossy(&o.stderr).contains(NO_EDITOR);
                            (
                                Ran::Exited {
                                    code: o.status.code(),
                                    no_editor,
                                },
                                o.stdout,
                                o.stderr,
                            )
                        }
                        Err(e) => (
                            Ran::Exited {
                                code: None,
                                no_editor: false,
                            },
                            Vec::new(),
                            format!("cannot run git: {}\n", e).into_bytes(),
                        ),
                    };
                    if let Some(pb) = &bars[i] {
                        pb.set_style(ProgressStyle::with_template("{msg}").unwrap());
                        let line = match &ran {
                            Ran::Exited { code: Some(0), .. } => {
                                output::result_line(Outcome::Ok, &order[i].name, color_err)
                            }
                            Ran::Exited { code, no_editor } => output::result_line(
                                Outcome::Fail,
                                &format!(
                                    "{}: exit {}{}",
                                    order[i].name,
                                    code.unwrap_or(-1),
                                    if *no_editor {
                                        " (--parallel: no terminal; run without it)"
                                    } else {
                                        ""
                                    }
                                ),
                                color_err,
                            ),
                            Ran::Skipped(_) => String::new(),
                        };
                        pb.finish_with_message(line);
                    }
                    {
                        let mut s = sched.lock().unwrap();
                        s.done[i] = true;
                    }
                    wake.notify_all();
                    let _ = tx.send(Done {
                        index: i,
                        ran,
                        stdout,
                        stderr,
                    });
                }
            });
        }
        drop(tx);
        for done in rx {
            let index = done.index;
            finished[index] = Some(done);
            print_ready(&mut finished, w, err)?;
        }
        Ok(())
    })?;
    if let Some(mp) = &mp {
        let _ = mp.clear();
    }
    Ok(finished
        .into_iter()
        .map(|d| {
            d.map(|d| d.ran).unwrap_or(Ran::Exited {
                code: None,
                no_editor: false,
            })
        })
        .collect())
}

/// The editor a parallel run gets: one that fails, saying why.
fn editor_refusal() -> String {
    format!("sh -c 'echo \"{}\" >&2; exit 1' --", NO_EDITOR)
}

/// The pager git would use for `command`, when it would page it: `pager.<cmd>`
/// first — a boolean, or a pager of its own — else the commands git pages
/// itself. `cat` or nothing is no pager.
fn pager_for(root: &Path, command: &str) -> Option<String> {
    let configured = crate::git::query(root, &["config", "--get", &format!("pager.{}", command)]);
    let wanted = match configured.as_deref().map(str::to_lowercase).as_deref() {
        Some("true" | "yes" | "on" | "1") => true,
        Some("false" | "no" | "off" | "0") => false,
        Some(_) => return configured.filter(|p| !p.trim().is_empty() && p.trim() != "cat"),
        None => PAGED.contains(&command),
    };
    if !wanted {
        return None;
    }
    let pager = crate::git::query(root, &["var", "GIT_PAGER"])?;
    let pager = pager.trim();
    (!pager.is_empty() && pager != "cat").then(|| pager.to_string())
}

/// Whether `e` is a write to a reader that went away.
fn broken_pipe(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::BrokenPipe)
}

/// The pager every repository's output goes to.
struct Pager {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
}

impl Pager {
    fn spawn(command: &str) -> Result<Pager> {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command).stdin(Stdio::piped());
        // As git sets them for its own pager.
        if std::env::var_os("LESS").is_none() {
            cmd.env("LESS", "FRX");
        }
        if std::env::var_os("LV").is_none() {
            cmd.env("LV", "-c");
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().expect("piped above");
        Ok(Pager { child, stdin })
    }

    fn finish(self) {
        let Pager { mut child, stdin } = self;
        drop(stdin);
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitscale_options_are_taken_up_to_the_separator() {
        let words = |list: &[&str]| -> Vec<OsString> { list.iter().map(OsString::from).collect() };
        let mut opts = Options::default();
        let rest = opts
            .take_from(&words(&[
                "--oneline",
                "--foreach",
                "--for",
                "a",
                "--for=b",
                "--parallel=3",
                "-s",
                "--force-sync",
                "--",
                "--foreach",
            ]))
            .unwrap();
        assert_eq!(rest, words(&["--oneline", "-s", "--", "--foreach"]));
        assert_eq!(
            opts,
            Options {
                select: vec!["a".into(), "b".into()],
                foreach: true,
                parallel: Some(3),
                force_sync: true,
            }
        );
        assert!(Options::default().take_from(&words(&["--for"])).is_err());
    }

    #[test]
    fn a_parallel_editor_fails_and_says_so() {
        let refusal = editor_refusal();
        let out = Command::new("sh")
            .arg("-c")
            .arg(format!("{} \"$@\"", refusal))
            .arg("editor")
            .arg("/tmp/x")
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains(NO_EDITOR));
    }
}

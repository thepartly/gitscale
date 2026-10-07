pub mod artefact;
pub mod cache;
pub mod checkout;
pub mod ci;
pub mod commands;
pub mod config;
pub mod exclude;
pub mod git;
pub mod gitlab;
pub mod hash;
pub mod hooks;
pub mod ledger;
pub mod man;
pub mod oci_layout;
pub mod output;
pub mod paths;
pub mod prefer;
pub mod progress;
pub mod promote;
pub mod registry;
pub mod remote_branch;
pub mod resolution;
pub mod resolve;
pub mod skill;
mod ssh;
pub mod store;
pub mod stores;
pub mod topic;
pub mod trust;
pub mod urls;
pub mod version;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use output::ColorChoice;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// An error already told on stderr in full: the command exits 1 and nothing
/// more is printed.
#[derive(Debug)]
pub struct Reported;

impl std::fmt::Display for Reported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("failed")
    }
}

impl std::error::Error for Reported {}

/// Fail with nothing more to say than what is already printed.
pub fn reported() -> anyhow::Error {
    anyhow::Error::new(Reported)
}

const AFTER_HELP: &str = "Any other command is a git command: it runs in the root and in every \
checkout on the topic, dependencies first and the root last, and the checkouts are placed again \
when it moved one. GitScale's --for, --foreach, --parallel and --force-sync go before or after \
the git command, up to `--`; every other option is git's.";

#[derive(Parser)]
#[command(
    name = "gitscale",
    version = VERSION,
    propagate_version = true,
    allow_external_subcommands = true,
    about = "GitScale — work across the repositories a .gitscale.toml declares, as one.",
    after_help = AFTER_HELP
)]
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// In CI, talk to remotes directly instead of through the CI cache
    #[arg(long, global = true)]
    no_cache: bool,

    /// Start the search for the workspace at PATH, and take directory
    /// arguments relative to it
    #[arg(short = 'C', long = "root", value_name = "PATH", global = true)]
    root: Option<PathBuf>,

    /// When to use colour and icons
    #[arg(long, value_enum, value_name = "WHEN", default_value_t = ColorChoice::Auto, global = true)]
    color: ColorChoice,

    /// Run a git command only in these repositories; `.` is the root.
    /// Repeatable. Also taken after the git command
    #[arg(long = "for", value_name = "DIR", help_heading = "Git commands")]
    select: Vec<String>,

    /// Run a git command in every checkout, not only those on the topic.
    /// Also taken after the git command
    #[arg(long, help_heading = "Git commands")]
    foreach: bool,

    /// Run a git command in parallel, at most N repositories at once
    /// (default: the number of CPUs, at most 16). Also taken after the git
    /// command
    #[arg(
        long,
        value_name = "N",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "0",
        help_heading = "Git commands"
    )]
    parallel: Option<usize>,

    /// After a git command, place as `sync --force` does. Also taken after
    /// the git command
    #[arg(long, help_heading = "Git commands")]
    force_sync: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show every checkout of the workspace: its revision, how it was chosen,
    /// and its state
    #[command(visible_alias = "list")]
    Ls {
        /// Resolve against the remotes first
        #[arg(long)]
        fetch: bool,
        #[arg(short, long, value_parser = ["table", "json"], default_value = "table")]
        format: String,
    },
    /// Show how checkouts got their revisions: every request, who made it,
    /// and which one won. With no directories, every checkout more than one
    /// repository asks for
    Explain {
        /// Resolve against the remotes first
        #[arg(long)]
        fetch: bool,
        dirs: Vec<String>,
    },
    /// Begin, go to, join, show and end topics: one branch name across the
    /// repositories a change touches. Alone, print the current topic
    Topic {
        #[command(subcommand)]
        action: Option<TopicAction>,
    },
    /// Raise pins on the topic: promote its released repositories, or raise
    /// named dependencies to their newest release
    Upgrade {
        /// Let a raise cross a major
        #[arg(long)]
        major: bool,
        /// Commit each edited .gitscale.toml, that file alone
        #[arg(long)]
        commit: bool,
        /// Print the plan and change nothing
        #[arg(long)]
        dry_run: bool,
        /// Dependencies to raise to their newest release, in the topic's
        /// configs that ask for them
        dirs: Vec<String>,
    },
    /// Put every checkout where resolution says, against the remotes as
    /// they are now: check out what is missing, relink, prune images, run
    /// post_sync
    Sync {
        /// Also relink checkouts with work, and remove what nothing needs
        /// although it holds work
        #[arg(long)]
        force: bool,
        dirs: Vec<String>,
    },
    /// Remove untracked files from the root and each checkout, keeping every
    /// checkout and link. Without -f, only lists them
    Clean {
        /// Only list what would go (also the default without -f)
        #[arg(short = 'n')]
        dry_run: bool,
        /// Delete
        #[arg(short = 'f')]
        force: bool,
        /// Untracked directories too
        #[arg(short = 'd')]
        directories: bool,
        /// Ignored files too
        #[arg(short = 'x', conflicts_with = "only_ignored")]
        ignored: bool,
        /// Only ignored files
        #[arg(short = 'X')]
        only_ignored: bool,
        /// Keep this, in .gitignore syntax, anchored at each repository's
        /// root. Repeatable
        #[arg(short = 'e', value_name = "PATTERN", allow_hyphen_values = true)]
        exclude: Vec<String>,
        /// Report only failures
        #[arg(short = 'q')]
        quiet: bool,
        dirs: Vec<String>,
    },
    /// Compact: git gc in every store, and drop the images nothing has used
    /// lately
    Gc {
        /// How recently an image must have been used to be kept, e.g.
        /// '6months' (default: [clean] keep_recent, else 3months)
        #[arg(long, value_name = "PERIOD")]
        keep_recent: Option<String>,
    },
    /// Add a dependency to the root's .gitscale.toml and check it out
    Require {
        dir: String,
        url: String,
        revision: Option<String>,
    },
    /// Remove a dependency from the root's .gitscale.toml
    Unrequire { dir: String },
    /// How checkouts arrive: their sources, or the published artefact of
    /// their release. Without a form, show the preferences. Only records:
    /// the next placement applies it
    Prefer {
        /// Their sources: the default, removing a preference
        #[arg(long, group = "form")]
        source: bool,
        /// The artefact of their release, in place of the sources
        #[arg(long, group = "form")]
        artefact: bool,
        dirs: Vec<String>,
    },
    /// The merge gate: fail while any checkout comes from a topic branch
    /// rather than a pinned revision
    Check,
    /// The source hash of the root, or of checkouts: what each one's own
    /// pipeline builds, from its tree and its dependencies' as it resolves
    /// them
    Hash {
        /// Hash each repository's commit, leaving uncommitted changes out
        #[arg(long)]
        committed: bool,
        #[arg(short, long, value_parser = ["text", "json"], default_value = "text")]
        format: String,
        dirs: Vec<String>,
    },
    /// Publish artefacts, and see what the registry holds for each entry
    Artefact {
        #[command(subcommand)]
        action: ArtefactAction,
    },
    /// Inspect and maintain the CI cache
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Install or inspect gitscale's git hooks
    Hook {
        #[command(subcommand)]
        action: HookAction,
    },
    /// Install, inspect or remove the agent skill that teaches coding agents
    /// the GitScale workflow
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    #[command(external_subcommand)]
    Git(Vec<OsString>),
}

#[derive(Subcommand)]
enum TopicAction {
    /// Put checkouts on the topic, from the commit each is at, writable; an
    /// artefact becomes a checkout of its source
    Join {
        /// Join the dependants instead: of each checkout named that ask for
        /// less than its newest release, or, with none named, of the
        /// topic's changes, one level up
        #[arg(long)]
        dependants: bool,
        /// Checkouts to join: their directory, or the path of a link a
        /// repository has to one
        dirs: Vec<String>,
    },
    /// Take checkouts off the topic: back at their pins, their topic
    /// branches deleted
    Leave { dirs: Vec<String> },
    /// Begin a topic: a new branch of the root, or a worktree of its own
    Start {
        /// The branch to start from (default: the remote's default branch)
        #[arg(long, value_name = "BRANCH")]
        from: Option<String>,
        /// In a worktree of its own
        #[arg(long, conflicts_with = "no_worktree")]
        worktree: bool,
        /// In this worktree
        #[arg(long)]
        no_worktree: bool,
        /// Where the worktree goes
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
        name: String,
    },
    /// Go to an existing branch of the root: a topic, a colleague's, or a
    /// pinned one
    Switch {
        /// In a worktree of its own
        #[arg(long, conflicts_with = "no_worktree")]
        worktree: bool,
        /// In this worktree
        #[arg(long)]
        no_worktree: bool,
        /// Where a new worktree goes
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
        name: String,
    },
    /// The current topic: its joined repositories, what each still needs,
    /// and what to merge next
    Status {
        /// Answer from what this machine has, without fetching the root and
        /// the joined repositories first
        #[arg(long)]
        offline: bool,
        /// The default: fetch first. Kept for scripts that pass it
        #[arg(long, hide = true, conflicts_with = "offline")]
        fetch: bool,
        #[arg(short, long, value_parser = ["table", "json"], default_value = "table")]
        format: String,
    },
    /// Every topic of the root: its worktree, joined checkouts and state
    List {
        /// Answer from what this machine has, without fetching the root and
        /// the joined checkouts first
        #[arg(long)]
        offline: bool,
        /// The default: fetch first. Kept for scripts that pass it
        #[arg(long, hide = true, conflicts_with = "offline")]
        fetch: bool,
        #[arg(short, long, value_parser = ["table", "json"], default_value = "table")]
        format: String,
    },
    /// End a merged topic: back on the default branch, or its worktree
    /// removed, and its branches deleted
    Finish {
        /// Drop a topic that is not merged
        #[arg(long)]
        force: bool,
        name: Option<String>,
    },
}

#[derive(Subcommand)]
enum ArtefactAction {
    /// Pack the files the [artefact] table selects and push them to the
    /// registry, as the image of the sources checked out: tagged with their
    /// source hash, and with RELEASE when one is given
    Publish {
        /// Replace an image already published for these sources, or a
        /// version tag naming another image
        #[arg(long)]
        force: bool,
        /// List what each layer would hold and its digest, without pushing
        #[arg(long)]
        dry_run: bool,
        /// Pack nothing: release the image already published for these
        /// sources — the branch build a squash merge kept
        #[arg(long, conflicts_with = "force", requires = "release")]
        reuse: bool,
        /// The release this commit is: the image is tagged with it
        release: Option<String>,
    },
    /// Show, for each checkout taken as an artefact, the image, the release
    /// it is taken at, whether that release is published, and what is
    /// installed
    Show { dirs: Vec<String> },
    /// List the releases each checkout taken as an artefact has images for,
    /// with the source hash of each
    List { dirs: Vec<String> },
}

#[derive(Subcommand)]
enum SkillAction {
    /// Write the skill to ~/.agents/skills/gitscale, and to
    /// ~/.claude/skills/gitscale when ~/.claude exists. Once installed, each
    /// interactive run keeps it current
    Install {
        /// Replace a copy edited by hand, or a file gitscale did not write
        #[arg(long)]
        force: bool,
    },
    /// Show where the skill is installed and which version
    Status,
    /// Remove the skill gitscale installed
    Remove {
        /// Remove a copy edited by hand too
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum CacheAction {
    /// Show what the cache holds: one line per repository, and every pin and
    /// image it is keeping
    Status,
    /// Add the pins and images a CI job of this workspace would take
    Update { dirs: Vec<String> },
    /// Evict what nothing has used lately
    Compact {
        /// How recently an entry must have been used to be kept, e.g. '2weeks'
        #[arg(long, value_name = "PERIOD", default_value = "12months")]
        keep_recent: String,
    },
}

#[derive(Subcommand)]
enum HookAction {
    /// Install gitscale's post-checkout and post-merge hooks, and its man
    /// pages
    Install {
        /// Install for every user on this machine (/etc/gitconfig)
        #[arg(long, conflicts_with_all = ["global", "local"])]
        system: bool,
        /// Install for the current user (~/.gitconfig)
        #[arg(long, conflicts_with_all = ["system", "local"])]
        global: bool,
        /// Install into this repository only (default)
        #[arg(long, conflicts_with_all = ["system", "global"])]
        local: bool,
        /// Replace an existing core.hooksPath or displaced hook
        #[arg(long)]
        force: bool,
        /// Repositories whose .gitscale.toml [hooks] commands this hook may
        /// run: comma-separated glob patterns matched against host/owner/repo,
        /// e.g. 'github.com/acme/*,git.internal.example/*'. Required for
        /// --global and --system; use '*' to allow every repository.
        #[arg(long, value_name = "PATTERNS")]
        allow: Option<String>,
        /// Hooks to install besides post-checkout and post-merge, which are
        /// always installed: comma-separated hook names, `lfs` for git-lfs's
        /// hooks, `all` for every one. With a --global or --system install no
        /// other hook runs in any repository. Re-installing without it keeps
        /// the hooks installed before.
        #[arg(long, value_name = "HOOKS")]
        hooks: Option<String>,
    },
    /// Remove gitscale's git hooks
    Uninstall {
        #[arg(long, conflicts_with_all = ["global", "local"])]
        system: bool,
        #[arg(long, conflicts_with_all = ["system", "local"])]
        global: bool,
        #[arg(long, conflicts_with_all = ["system", "global"])]
        local: bool,
    },
    /// Show where hooks are installed and whether anything shadows them
    Status,
    /// Run a git hook: the repository's own, git-lfs, .githooks/ and the
    /// placement (invoked by the installed hook)
    Run {
        name: String,
        /// The hook fired in this child: placement leaves it where git put it.
        /// Passed by hooks an earlier gitscale installed
        #[arg(long, value_name = "PATH")]
        child: Option<PathBuf>,
        /// The installed hook that ran this; git's arguments follow `--`
        #[arg(long, value_name = "PATH", requires = "scope")]
        shim: Option<PathBuf>,
        /// The scope the hook was installed at: local, global or system
        #[arg(long, value_name = "SCOPE", requires = "shim")]
        scope: Option<String>,
        /// git's arguments to the hook
        #[arg(last = true, value_name = "ARGS")]
        args: Vec<OsString>,
    },
}

fn hook_scope(system: bool, global: bool, _local: bool) -> commands::hook::Scope {
    if system {
        commands::hook::Scope::System
    } else if global {
        commands::hook::Scope::Global
    } else {
        commands::hook::Scope::Local
    }
}

/// The command line as clap knows it: what the man pages are made from.
pub fn command() -> clap::Command {
    Cli::command()
}

pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

/// How one run reaches the user.
#[derive(Debug, Clone, Copy)]
pub struct Io {
    /// Multi-repository commands draw live progress on stderr.
    pub interactive: bool,
    /// Git commands run with this process's stdin, stdout and stderr, rather
    /// than having their output collected into the command's.
    pub inherit: bool,
    pub streams: output::Streams,
}

/// The binaries' entry point. `insert` is the subcommand a `git-<name>`
/// binary stands for: `git topic join x` runs `gitscale topic join x`.
pub fn main(insert: Option<&str>) -> i32 {
    use std::io::IsTerminal;
    let mut args: Vec<OsString> = std::env::args_os().collect();
    let invoked = args
        .first()
        .and_then(|a| {
            std::path::Path::new(a)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let bin = match insert {
        Some(name) => {
            args.insert(1, name.into());
            "git"
        }
        None if invoked == "git-scale" => "git scale",
        None => "gitscale",
    };
    if let Some(first) = args.first_mut() {
        *first = bin.into();
    }
    let streams = output::Streams {
        stdout_tty: std::io::stdout().is_terminal(),
        stderr_tty: std::io::stderr().is_terminal(),
    };
    let io = Io {
        interactive: streams.stdout_tty,
        inherit: true,
        streams,
    };
    let code = run_args(args, io, &mut std::io::stdout(), &mut std::io::stderr());
    let _ = std::io::stdout().flush();
    code
}

pub fn run_cli(args: &[&str]) -> CliOutput {
    run_cli_with(args, progress::is_interactive())
}

/// Run a command line in this process, its output collected. Tests use this,
/// with the interactive/plain rendering chosen explicitly rather than sniffed
/// from the process's stdout, so their captured output does not depend on
/// whether the harness happens to run under a TTY. Git commands it forwards
/// have their output collected too.
pub fn run_cli_with(args: &[&str], interactive: bool) -> CliOutput {
    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    let io = Io {
        interactive,
        inherit: false,
        streams: output::Streams::default(),
    };
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    let code = run_args(args, io, &mut stdout_buf, &mut stderr_buf);
    CliOutput {
        stdout: String::from_utf8_lossy(&stdout_buf).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_buf).into_owned(),
        success: code == 0,
    }
}

/// Whether a person is watching this run: at a terminal, and neither in CI
/// nor run by a git hook — which may inherit the terminal of the git command
/// that fired it, with nobody expecting a prompt.
fn watched(io: Io, ci: bool, hook: bool) -> bool {
    (io.interactive || io.streams.stdout_tty || io.streams.stderr_tty) && !ci && !hook
}

/// Run a command line: the process's exit status.
fn run_args(args: Vec<OsString>, io: Io, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind;
            let success = matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion);
            if success {
                let _ = write!(out, "{}", e);
                return 0;
            }
            let _ = write!(err, "{}", e);
            return 1;
        }
    };
    output::init(cli.color, io.streams);
    git::set_watched(watched(
        io,
        git::is_ci(),
        std::env::var_os("GITSCALE_HOOK").is_some_and(|v| !v.is_empty()) || is_hook_run(&cli),
    ));
    match run_cli_inner(cli, io, out, err) {
        Ok(()) => 0,
        Err(e) if e.is::<Reported>() => 1,
        Err(e) => match e.downcast_ref::<commands::hook::HookExit>() {
            Some(exit) => exit.0,
            None => {
                let _ = writeln!(
                    err,
                    "{} {}",
                    output::paint(output::stderr(), output::BOLD_RED, "Error:"),
                    e
                );
                1
            }
        },
    }
}

/// `hook run`: started by git, for every hook in every repository, with the
/// terminal of whatever git command fired it.
fn is_hook_run(cli: &Cli) -> bool {
    matches!(
        cli.command,
        Commands::Hook {
            action: HookAction::Run { .. }
        }
    )
}

fn run_cli_inner(cli: Cli, io: Io, out: &mut dyn Write, err: &mut dyn Write) -> Result<()> {
    let is_skill = matches!(cli.command, Commands::Skill { .. });
    let is_hook = is_hook_run(&cli);
    // Where a sync and a table may point at `skill install`.
    let hint = match &cli.command {
        Commands::Sync { .. } => true,
        Commands::Ls { format, .. } => format == "table",
        _ => false,
    };
    let start = cli.root.clone();

    let result = run_command(cli, io, out, err);
    // Only on a run somebody is watching, never in CI, and never from a git
    // hook, which fires on every commit.
    if io.interactive && !git::is_ci() && !is_hook {
        // Installed man pages follow the binary; they are never created here.
        man::refresh();
        if let (Some(home), false) = (skill::home(), is_skill) {
            // An installed skill follows the binary; it is never created here.
            for path in skill::refresh(&home) {
                let _ = writeln!(
                    err,
                    "  updated the gitscale agent skill at {}",
                    path.display()
                );
            }
            if let (Ok(()), true) = (&result, hint) {
                if let Ok(root) = config::find_root(start.as_deref()) {
                    skill::hint(&home, &root, err);
                }
            }
        }
    }
    result
}

fn run_command(cli: Cli, io: Io, out: &mut dyn Write, err: &mut dyn Write) -> Result<()> {
    let verbose = cli.verbose;
    let no_cache = cli.no_cache;
    let interactive = io.interactive;
    let root = cli.root.as_deref();
    let forwarding =
        !cli.select.is_empty() || cli.foreach || cli.parallel.is_some() || cli.force_sync;
    if forwarding && !matches!(cli.command, Commands::Git(_)) {
        anyhow::bail!(
            "--for, --foreach, --parallel and --force-sync are for git commands; GitScale's \
             own commands take their own options"
        );
    }

    match cli.command {
        Commands::Git(args) => commands::forward::run(
            root,
            &commands::forward::Options {
                select: cli.select.clone(),
                foreach: cli.foreach,
                parallel: cli.parallel,
                force_sync: cli.force_sync,
            },
            &args,
            verbose,
            no_cache,
            io,
            out,
            err,
        ),
        Commands::Ls { fetch, format } => commands::ls::run(
            root,
            fetch,
            &format,
            verbose,
            no_cache,
            io.interactive,
            out,
            err,
        ),
        Commands::Explain { fetch, dirs } => {
            commands::ls::explain(root, fetch, &dirs, verbose, no_cache, out, err)
        }
        Commands::Topic { action } => {
            let action = match action {
                None => commands::topic::Action::Print,
                Some(TopicAction::Join { dirs, dependants }) => {
                    commands::topic::Action::Join { dirs, dependants }
                }
                Some(TopicAction::Leave { dirs }) => commands::topic::Action::Leave(dirs),
                Some(TopicAction::Start {
                    from,
                    worktree,
                    no_worktree,
                    dir,
                    name,
                }) => commands::topic::Action::Start {
                    name,
                    from,
                    worktree: worktree_choice(worktree, no_worktree),
                    dir,
                },
                Some(TopicAction::Switch {
                    worktree,
                    no_worktree,
                    dir,
                    name,
                }) => commands::topic::Action::Switch {
                    name,
                    worktree: worktree_choice(worktree, no_worktree),
                    dir,
                },
                Some(TopicAction::Status {
                    offline, format, ..
                }) => commands::topic::Action::Status {
                    fetch: !offline,
                    format,
                },
                Some(TopicAction::List {
                    offline, format, ..
                }) => commands::topic::Action::List {
                    fetch: !offline,
                    format,
                },
                Some(TopicAction::Finish { force, name }) => {
                    commands::topic::Action::Finish { name, force }
                }
            };
            commands::topic::run(root, action, verbose, no_cache, interactive, out, err)
        }
        Commands::Upgrade {
            major,
            commit,
            dry_run,
            dirs,
        } => commands::upgrade::run(
            root,
            &commands::upgrade::Options {
                dirs: &dirs,
                major,
                commit,
                dry_run,
            },
            verbose,
            no_cache,
            out,
        ),
        Commands::Sync { force, dirs } => {
            commands::sync::run(root, &dirs, verbose, no_cache, force, interactive, out, err)
        }
        Commands::Clean {
            dry_run,
            force,
            directories,
            ignored,
            only_ignored,
            exclude,
            quiet,
            dirs,
        } => commands::clean::run(
            root,
            &commands::clean::Options {
                delete: force && !dry_run,
                directories,
                ignored: if only_ignored {
                    commands::clean::Ignored::Only
                } else if ignored {
                    commands::clean::Ignored::Too
                } else {
                    commands::clean::Ignored::Kept
                },
                exclude: &exclude,
                quiet,
                dirs: &dirs,
            },
            interactive,
            out,
            err,
        ),
        Commands::Gc { keep_recent } => commands::clean::gc(root, keep_recent.as_deref(), out),
        Commands::Require { dir, url, revision } => commands::require::require(
            root,
            &dir,
            &url,
            revision.as_deref().unwrap_or_default(),
            verbose,
            no_cache,
            interactive,
            out,
            err,
        ),
        Commands::Unrequire { dir } => {
            commands::require::unrequire(root, &dir, verbose, no_cache, interactive, out, err)
        }
        Commands::Check => commands::check::run(root, verbose, no_cache, out),
        Commands::Prefer {
            source,
            artefact,
            dirs,
        } => {
            use crate::prefer::Form;
            let form = match (source, artefact) {
                (true, _) => Some(Form::Source),
                (_, true) => Some(Form::Artefact),
                _ => None,
            };
            commands::prefer::run(root, form, &dirs, no_cache, out)
        }
        Commands::Hash {
            committed,
            format,
            dirs,
        } => commands::hash::run(root, &dirs, committed, &format, verbose, no_cache, out),
        Commands::Artefact { action } => match action {
            ArtefactAction::Publish {
                force,
                dry_run,
                reuse,
                release,
            } => commands::artefact::publish(root, release.as_deref(), force, dry_run, reuse, out),
            ArtefactAction::Show { dirs } => commands::artefact::show(root, &dirs, out),
            ArtefactAction::List { dirs } => commands::artefact::list(root, &dirs, out),
        },
        Commands::Cache { action } => match action {
            CacheAction::Status => commands::cache::status(root, out),
            CacheAction::Update { dirs } => {
                commands::cache::update(root, &dirs, verbose, interactive, out, err)
            }
            // The cache belongs to the user, not to a workspace: -C changes
            // nothing.
            CacheAction::Compact { keep_recent } => commands::cache::compact(&keep_recent, out),
        },
        Commands::Skill { action } => {
            let home = skill::home()
                .ok_or_else(|| anyhow::anyhow!("HOME is not set: nowhere to put the skill"))?;
            match action {
                SkillAction::Install { force } => {
                    for (path, before) in skill::install(&home, force)? {
                        if before == skill::State::Current {
                            writeln!(out, "  ok       {} (already current)", path.display())?;
                        } else {
                            writeln!(out, "  install  {}", path.display())?;
                        }
                    }
                    Ok(())
                }
                SkillAction::Status => {
                    for (path, state) in skill::status(&home) {
                        writeln!(out, "{}: {}", path.display(), state.describe())?;
                    }
                    Ok(())
                }
                SkillAction::Remove { force } => {
                    let removed = skill::remove(&home, force)?;
                    if removed.is_empty() {
                        writeln!(out, "Nothing to remove: the skill is not installed.")?;
                    }
                    for path in removed {
                        writeln!(out, "  remove  {}", path.display())?;
                    }
                    Ok(())
                }
            }
        }
        Commands::Hook { action } => match action {
            HookAction::Install {
                system,
                global,
                local,
                force,
                allow,
                hooks,
            } => commands::hook::install(
                hook_scope(system, global, local),
                root,
                force,
                allow.as_deref(),
                hooks.as_deref(),
                out,
            ),
            HookAction::Uninstall {
                system,
                global,
                local,
            } => commands::hook::uninstall(hook_scope(system, global, local), root, out),
            HookAction::Status => commands::hook::status(root, out),
            HookAction::Run {
                name,
                child,
                shim,
                scope,
                args,
            } => {
                let scope = scope
                    .as_deref()
                    .map(commands::hook::Scope::parse)
                    .transpose()?;
                let handoff = match (shim.as_deref(), scope) {
                    (Some(shim), Some(scope)) => Some(commands::hook::Handoff {
                        shim,
                        scope,
                        args: &args,
                    }),
                    _ => None,
                };
                commands::hook::run(
                    &name,
                    root,
                    child.as_deref(),
                    handoff,
                    verbose,
                    no_cache,
                    interactive,
                    out,
                    err,
                )
            }
        },
    }
}

/// `--worktree` / `--no-worktree`: `None` when neither is given.
fn worktree_choice(worktree: bool, no_worktree: bool) -> Option<bool> {
    match (worktree, no_worktree) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command run at a terminal is watched, so git keeps the user's
    /// askpass helpers; with no terminal, in CI, or from a git hook — which
    /// may have the terminal of the git command that fired it — it is not.
    #[test]
    fn only_a_person_at_a_terminal_is_watched() {
        let io = |stderr_tty| Io {
            interactive: false,
            inherit: true,
            streams: output::Streams {
                stdout_tty: false,
                stderr_tty,
            },
        };
        assert!(watched(io(true), false, false));
        assert!(!watched(io(false), false, false));
        assert!(!watched(io(true), true, false), "in CI");
        assert!(!watched(io(true), false, true), "from a git hook");
    }

    /// Each GitScale command line in the skill — `git scale …`, `git topic
    /// …`, `git upgrade …`, `git explain …`, `gitscale …` — as the words that
    /// name the command after `gitscale`, and the long flags it uses.
    fn skill_commands() -> Vec<(Vec<String>, Vec<String>)> {
        let text = include_str!("skill.md");
        let mut found = Vec::new();
        for (i, span) in text.split('`').enumerate() {
            if i % 2 == 0 {
                continue;
            }
            for part in span.split("&&") {
                let words: Vec<&str> = part.split_whitespace().collect();
                let rest: Vec<&str> = match words.as_slice() {
                    ["git", "scale", rest @ ..] | ["gitscale", rest @ ..] => rest.to_vec(),
                    ["git", sub @ ("topic" | "upgrade" | "explain"), rest @ ..] => {
                        std::iter::once(*sub).chain(rest.iter().copied()).collect()
                    }
                    _ => continue,
                };
                let (mut path, mut flags) = (Vec::new(), Vec::new());
                // The command's words come first; after any argument, only
                // long flags are taken.
                let mut in_path = true;
                for word in rest {
                    let word = word.trim_matches(|c| c == '[' || c == ']');
                    if word.starts_with("--") {
                        flags.push(word.to_string());
                    } else if in_path && word.starts_with(|c: char| c.is_ascii_lowercase()) {
                        path.push(word.to_string());
                    } else {
                        in_path = false;
                    }
                    in_path &= flags.is_empty();
                }
                found.push((path, flags));
            }
        }
        found
    }

    #[test]
    fn every_command_the_skill_names_exists() {
        let commands = skill_commands();
        assert!(commands.len() > 10, "{:?}", commands);
        let ours: Vec<String> = Cli::command()
            .get_subcommands()
            .flat_map(|c| {
                std::iter::once(c.get_name().to_string())
                    .chain(c.get_all_aliases().map(str::to_string))
            })
            .collect();
        for (path, flags) in commands {
            // A git command: forwarded, and git's to know.
            if path.is_empty() || !ours.contains(&path[0]) {
                continue;
            }
            // Words after a command that takes no subcommand are arguments.
            let mut args = vec!["gitscale".to_string()];
            let mut command = Cli::command();
            for word in &path {
                match command.find_subcommand(word) {
                    Some(sub) => {
                        args.push(word.clone());
                        command = sub.clone();
                    }
                    None => break,
                }
            }
            args.push("--help".to_string());
            let help = match Cli::try_parse_from(&args) {
                Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => e.to_string(),
                other => panic!(
                    "{}: not a command ({:?})",
                    path.join(" "),
                    other.err().map(|e| e.kind())
                ),
            };
            for flag in flags {
                assert!(
                    help.contains(&flag),
                    "gitscale {} has no {}",
                    path.join(" "),
                    flag
                );
            }
        }
    }

    #[test]
    fn every_doc_the_skill_links_exists() {
        let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs");
        let mut linked = 0;
        for line in include_str!("skill.md").lines() {
            let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
            if let Some(doc) = cells.last().filter(|c| c.ends_with(".md")) {
                assert!(docs.join(doc).is_file(), "docs/{} does not exist", doc);
                linked += 1;
            }
        }
        assert!(linked > 10);
    }
}

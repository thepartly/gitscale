pub mod artefact;
pub mod cache;
pub mod checkout;
pub mod ci;
pub mod commands;
pub mod config;
pub mod git;
pub mod gitlab;
pub mod hooks;
pub mod ledger;
pub mod oci_layout;
pub mod progress;
pub mod promote;
pub mod registry;
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
use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::PathBuf;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "gitscale", version = VERSION, about = "GitScale — manage multiple sub-repositories from a .gitscale.toml config.")]
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// In CI, talk to remotes directly instead of through the cache
    #[arg(long, global = true)]
    no_cache: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Fetch latest remote state for sub-repositories
    Fetch {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        names: Vec<String>,
    },
    /// Put every sub-repository where resolution says, cloning what is missing
    Pull {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        names: Vec<String>,
    },
    /// Push local changes for sub-repositories
    Push {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        names: Vec<String>,
    },
    /// Full sync: pull, relink, push
    Sync {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        #[arg(long)]
        force: bool,
        names: Vec<String>,
    },
    /// Commit local changes across sub-repositories with one shared message
    Commit {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        #[arg(short = 'm', long)]
        message: String,
        names: Vec<String>,
    },
    /// Remove untracked files from the workspace and its sub-repositories
    Clean {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        /// Actually delete. Without it, clean only lists what would go.
        #[arg(short = 'f', long)]
        force: bool,
        /// A path to keep, in .gitignore syntax, anchored at each repo's root.
        /// Applies to every repo cleaned; repeatable. Patterns that belong to
        /// one repo go in that repo's own [clean] exclude instead.
        #[arg(short = 'e', long = "exclude", value_name = "PATTERN")]
        exclude: Vec<String>,
        /// Compact instead: git gc in every store of the root, and drop the
        /// images nothing has used lately
        #[arg(long, conflicts_with_all = ["force", "exclude", "names"])]
        gc: bool,
        /// With --gc: how recently an image must have been used to be kept,
        /// e.g. '6months' (default: [clean] keep_recent, else 3months)
        #[arg(long, value_name = "PERIOD", requires = "gc")]
        keep_recent: Option<String>,
        names: Vec<String>,
    },
    /// Show status of repos declared in .gitscale.toml
    Status {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        #[arg(long)]
        fetch: bool,
        #[arg(short, long, value_parser = ["table", "json"], default_value = "table")]
        format: String,
        /// Show how each checkout got its revision: every request, who made
        /// it, and which one won. With no directories, every checkout more
        /// than one repository asks for
        #[arg(long, num_args = 0.., value_name = "DIR")]
        why: Option<Vec<String>>,
    },
    /// Put checkouts on the workspace's topic — the root's current branch —
    /// from the commit each is at, writable
    Develop {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        /// Take the checkouts off the topic instead: back at their pins,
        /// their topic branches deleted
        #[arg(long)]
        stop: bool,
        /// Checkouts to develop: their directory, or the path of a link a
        /// repository has to one
        dirs: Vec<String>,
    },
    /// Raise pins: promote a topic's released repositories, raise named
    /// dependencies to their newest release, or write what resolution
    /// selected into the root config
    Upgrade {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        /// Write the revision resolution selected into the root's own
        /// entries, with no tag lookup
        #[arg(long)]
        resolved: bool,
        /// Let a raise cross a semver major
        #[arg(long)]
        major: bool,
        /// Commit each edited .gitscale.toml, that file alone
        #[arg(long)]
        commit: bool,
        /// Print the plan and change nothing
        #[arg(long)]
        dry_run: bool,
        /// The topic to create when none is active and a repository other
        /// than the root has to be edited
        #[arg(short = 'c', long = "create", value_name = "BRANCH")]
        create: Option<String>,
        /// Dependencies to raise to their newest release, in every config
        /// that asks for them
        dirs: Vec<String>,
    },
    /// The merge gate: fail while any checkout comes from a topic branch
    /// rather than a pinned revision
    Check {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
    },
    /// Add a sub-repository entry to .gitscale config
    Add {
        directory: String,
        repo_url: String,
        revision: String,
        /// Use the repository's published artefact: instead of a checkout
        /// (replace), or laid over one (overlay)
        #[arg(long, value_parser = ["replace", "overlay"])]
        artefact: Option<String>,
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
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
    /// Remove a sub-repository entry from .gitscale config
    Remove {
        directory: String,
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ArtefactAction {
    /// Pack the files the [artefact] table selects and push them to the
    /// registry, as the image for one commit
    Publish {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        /// The commit to publish for (full SHA). Default: the CI job's
        /// commit, else HEAD
        #[arg(long, value_name = "SHA")]
        commit: Option<String>,
        /// Replace an image already published for this commit with different
        /// files
        #[arg(long)]
        force: bool,
        /// List what each layer would hold and its digest, without pushing
        #[arg(long)]
        dry_run: bool,
    },
    /// Show, for each artefact entry, the image, the commit its revision
    /// names now, whether that commit is published, and what is installed
    Show {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        names: Vec<String>,
    },
    /// List the commits each artefact entry has images for in the registry
    List {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        names: Vec<String>,
    },
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
    Status {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
    },
    /// Add the pins and images a CI job of this workspace would take
    Update {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        names: Vec<String>,
    },
    /// Evict what nothing has used lately
    Compact {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        /// How recently an entry must have been used to be kept, e.g. '2weeks'
        #[arg(long, value_name = "PERIOD", default_value = "12months")]
        keep_recent: String,
    },
}

#[derive(Subcommand)]
enum HookAction {
    /// Install gitscale's post-checkout and post-merge hooks
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
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
    },
    /// Remove gitscale's git hooks
    Uninstall {
        #[arg(long, conflicts_with_all = ["global", "local"])]
        system: bool,
        #[arg(long, conflicts_with_all = ["system", "local"])]
        global: bool,
        #[arg(long, conflicts_with_all = ["system", "global"])]
        local: bool,
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
    },
    /// Show where hooks are installed and whether anything shadows them
    Status {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
    },
    /// Run the pull for a git hook (invoked by the installed hook)
    Run {
        name: String,
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
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

pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

pub fn run_cli(args: &[&str]) -> CliOutput {
    run_cli_with(args, progress::is_interactive())
}

/// Like [`run_cli`], but with the interactive/plain rendering chosen explicitly
/// rather than sniffed from the process's stdout. Tests use this so their
/// captured output does not depend on whether the harness happens to run under
/// a TTY (which would otherwise switch the multi-repo commands to parallel
/// progress bars on stderr and leave the captured stdout empty).
pub fn run_cli_with(args: &[&str], interactive: bool) -> CliOutput {
    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();

    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind;
            let success = matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion);
            if success {
                let _ = write!(stdout_buf, "{}", e);
            } else {
                let _ = write!(stderr_buf, "{}", e);
            }
            return CliOutput {
                stdout: String::from_utf8_lossy(&stdout_buf).into_owned(),
                stderr: String::from_utf8_lossy(&stderr_buf).into_owned(),
                success,
            };
        }
    };

    let success = match run_cli_inner(cli, interactive, &mut stdout_buf, &mut stderr_buf) {
        Ok(()) => true,
        Err(e) => {
            let _ = writeln!(stderr_buf, "Error: {}", e);
            false
        }
    };

    CliOutput {
        stdout: String::from_utf8_lossy(&stdout_buf).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_buf).into_owned(),
        success,
    }
}

fn run_cli_inner(
    cli: Cli,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let is_skill = matches!(cli.command, Commands::Skill { .. });
    // Where `pull` and a status table may point at `skill install`.
    let hint_root = match &cli.command {
        Commands::Pull { root, .. } => Some(root.clone()),
        Commands::Status {
            root, format, why, ..
        } if format == "table" && why.is_none() => Some(root.clone()),
        _ => None,
    };

    let result = run_command(cli, interactive, out, err);
    // Only on a run somebody is watching, and never in CI.
    if interactive && !is_skill && !git::is_ci() {
        if let Some(home) = skill::home() {
            // An installed skill follows the binary; it is never created here.
            for path in skill::refresh(&home) {
                let _ = writeln!(
                    err,
                    "  updated the gitscale agent skill at {}",
                    path.display()
                );
            }
            if let (Ok(()), Some(start)) = (&result, hint_root) {
                if let Ok((_, root)) = config::load_workspace(start.as_deref()) {
                    skill::hint(&home, &root, err);
                }
            }
        }
    }
    result
}

fn run_command(
    cli: Cli,
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let verbose = cli.verbose;
    let no_cache = cli.no_cache;

    match cli.command {
        Commands::Fetch { root, names } => commands::fetch::run(
            root.as_deref(),
            &names,
            verbose,
            no_cache,
            interactive,
            out,
            err,
        ),
        Commands::Pull { root, names } => commands::pull::run(
            root.as_deref(),
            &names,
            verbose,
            no_cache,
            interactive,
            out,
            err,
        ),
        Commands::Push { root, names } => {
            commands::push::run(root.as_deref(), &names, interactive, out, err)
        }
        Commands::Sync { root, force, names } => commands::sync::run(
            root.as_deref(),
            &names,
            verbose,
            no_cache,
            force,
            interactive,
            out,
            err,
        ),
        Commands::Commit {
            root,
            message,
            names,
        } => commands::commit::run(root.as_deref(), &names, &message, interactive, out, err),
        Commands::Clean {
            root,
            force,
            exclude,
            gc,
            keep_recent,
            names,
        } => commands::clean::run(
            root.as_deref(),
            &names,
            &exclude,
            gc,
            keep_recent.as_deref(),
            force,
            interactive,
            out,
            err,
        ),
        Commands::Status {
            root,
            fetch,
            format,
            why,
        } => commands::status::run(
            root.as_deref(),
            fetch,
            &format,
            why.as_deref(),
            verbose,
            no_cache,
            out,
            err,
        ),
        Commands::Develop { root, stop, dirs } => {
            commands::develop::run(root.as_deref(), &dirs, stop, verbose, out)
        }
        Commands::Upgrade {
            root,
            resolved,
            major,
            commit,
            dry_run,
            create,
            dirs,
        } => commands::upgrade::run(
            root.as_deref(),
            &commands::upgrade::Options {
                dirs: &dirs,
                resolved,
                major,
                commit,
                dry_run,
                create: create.as_deref(),
            },
            verbose,
            no_cache,
            out,
        ),
        Commands::Check { root } => commands::check::run(root.as_deref(), verbose, no_cache, out),
        Commands::Add {
            directory,
            repo_url,
            revision,
            artefact,
            root,
        } => commands::add::run(
            &directory,
            &repo_url,
            &revision,
            artefact.as_deref(),
            root.as_deref(),
            out,
        ),
        Commands::Remove { directory, root } => {
            commands::remove::run(&directory, root.as_deref(), out)
        }
        Commands::Artefact { action } => match action {
            ArtefactAction::Publish {
                root,
                commit,
                force,
                dry_run,
            } => {
                commands::artefact::publish(root.as_deref(), commit.as_deref(), force, dry_run, out)
            }
            ArtefactAction::Show { root, names } => {
                commands::artefact::show(root.as_deref(), &names, out)
            }
            ArtefactAction::List { root, names } => {
                commands::artefact::list(root.as_deref(), &names, out)
            }
        },
        Commands::Cache { action } => match action {
            CacheAction::Status { root } => commands::cache::status(root.as_deref(), out),
            CacheAction::Update { root, names } => {
                commands::cache::update(root.as_deref(), &names, verbose, interactive, out, err)
            }
            // The cache belongs to the user, not to a workspace: -C is accepted
            // for symmetry with the other cache commands and changes nothing.
            CacheAction::Compact {
                root: _,
                keep_recent,
            } => commands::cache::compact(&keep_recent, out),
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
                root,
            } => commands::hook::install(
                hook_scope(system, global, local),
                root.as_deref(),
                force,
                allow.as_deref(),
                out,
            ),
            HookAction::Uninstall {
                system,
                global,
                local,
                root,
            } => commands::hook::uninstall(hook_scope(system, global, local), root.as_deref(), out),
            HookAction::Status { root } => commands::hook::status(root.as_deref(), out),
            HookAction::Run { name, root } => commands::hook::run(
                &name,
                root.as_deref(),
                verbose,
                no_cache,
                interactive,
                out,
                err,
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each `gitscale ...` span in the skill, as the words that name the
    /// command and the long flags it uses.
    fn skill_commands() -> Vec<(Vec<String>, Vec<String>)> {
        let text = include_str!("skill.md");
        let mut found = Vec::new();
        for (i, span) in text.split('`').enumerate() {
            if i % 2 == 0 {
                continue;
            }
            for part in span.split("&&") {
                let mut words = part.split_whitespace();
                if words.next() != Some("gitscale") {
                    continue;
                }
                let (mut path, mut flags) = (Vec::new(), Vec::new());
                // The command's words come first; after any argument, only
                // long flags are taken.
                let mut in_path = true;
                for word in words {
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
        for (path, flags) in commands {
            if path.is_empty() {
                continue;
            }
            let mut args = vec!["gitscale"];
            args.extend(path.iter().map(String::as_str));
            args.push("--help");
            let help = match Cli::try_parse_from(&args) {
                Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => e.to_string(),
                other => panic!(
                    "gitscale {}: not a command ({:?})",
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

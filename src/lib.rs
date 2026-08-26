pub mod commands;
pub mod config;
pub mod git;
pub mod hooks;
pub mod progress;
pub mod resolve;
pub mod storage;
pub mod urls;

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

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Clone sub-repositories from .gitscale config
    Clone {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        names: Vec<String>,
    },
    /// Fetch latest remote state for sub-repositories
    Fetch {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        names: Vec<String>,
    },
    /// Pull latest changes for sub-repositories
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
    /// Full sync: clone + pull + push
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
    /// Show status of repos declared in .gitscale.toml
    Status {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        #[arg(long)]
        fetch: bool,
        #[arg(short, long, value_parser = ["table", "json"], default_value = "table")]
        format: String,
    },
    /// Add a sub-repository entry to .gitscale config
    Add {
        directory: String,
        repo_url: String,
        revision: String,
        #[arg(long, value_parser = ["readonly", "readwrite", "artefact"], default_value = "readwrite")]
        mode: String,
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
    },
    /// Remove a sub-repository entry from .gitscale config
    Remove {
        directory: String,
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
    },
}

pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

pub fn run_cli(args: &[&str]) -> CliOutput {
    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    let interactive = progress::is_interactive();

    let success = match run_cli_inner(args, interactive, &mut stdout_buf, &mut stderr_buf) {
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
    args: &[&str],
    interactive: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let cli = Cli::try_parse_from(args)?;
    let verbose = cli.verbose;

    match cli.command {
        Commands::Clone { root, names } => {
            commands::clone::run(root.as_deref(), &names, verbose, interactive, out, err)
        }
        Commands::Fetch { root, names } => {
            commands::fetch::run(root.as_deref(), &names, interactive, out, err)
        }
        Commands::Pull { root, names } => {
            commands::pull::run(root.as_deref(), &names, verbose, interactive, out, err)
        }
        Commands::Push { root, names } => {
            commands::push::run(root.as_deref(), &names, verbose, interactive, out, err)
        }
        Commands::Sync { root, force, names } => {
            commands::sync::run(root.as_deref(), &names, verbose, force, interactive, out, err)
        }
        Commands::Commit {
            root,
            message,
            names,
        } => commands::commit::run(root.as_deref(), &names, &message, interactive, out, err),
        Commands::Status {
            root,
            fetch,
            format,
        } => commands::status::run(root.as_deref(), fetch, &format, verbose, out, err),
        Commands::Add {
            directory,
            repo_url,
            revision,
            mode,
            root,
        } => commands::add::run(
            &directory,
            &repo_url,
            &revision,
            &mode,
            root.as_deref(),
            out,
        ),
        Commands::Remove { directory, root } => {
            commands::remove::run(&directory, root.as_deref(), out)
        }
    }
}

mod commands;
mod config;
mod git;
mod hooks;
mod storage;
mod urls;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process;

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
        /// Root directory containing .gitscale.toml (default: auto-detect)
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        /// Clone only these entries (default: all)
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
        names: Vec<String>,
    },
    /// Show status of repos declared in .gitscale.toml
    Status {
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
        /// Run git fetch before checking status
        #[arg(long)]
        fetch: bool,
        /// Output format
        #[arg(short, long, value_parser = ["table", "json"], default_value = "table")]
        format: String,
    },
    /// Add a sub-repository entry to .gitscale config
    Add {
        /// Local subdirectory name
        directory: String,
        /// Git repository URL
        repo_url: String,
        /// Branch, tag, or commit to checkout
        revision: String,
        /// Access mode for the sub-repository
        #[arg(long, value_parser = ["readonly", "readwrite", "artefact"], default_value = "readwrite")]
        mode: String,
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
    },
    /// Remove a sub-repository entry from .gitscale config
    Remove {
        /// Local subdirectory name to remove
        directory: String,
        #[arg(short = 'C', long)]
        root: Option<PathBuf>,
    },
}

fn main() {
    let cli = Cli::parse();
    let verbose = cli.verbose;

    let result = match cli.command {
        Commands::Clone { root, names } => commands::clone::run(root.as_deref(), &names, verbose),
        Commands::Fetch { root, names } => commands::fetch::run(root.as_deref(), &names, verbose),
        Commands::Pull { root, names } => commands::pull::run(root.as_deref(), &names, verbose),
        Commands::Push { root, names } => commands::push::run(root.as_deref(), &names, verbose),
        Commands::Sync { root, names } => commands::sync::run(root.as_deref(), &names, verbose),
        Commands::Status {
            root,
            fetch,
            format,
        } => commands::status::run(root.as_deref(), fetch, &format, verbose),
        Commands::Add {
            directory,
            repo_url,
            revision,
            mode,
            root,
        } => commands::add::run(&directory, &repo_url, &revision, &mode, root.as_deref()),
        Commands::Remove { directory, root } => commands::remove::run(&directory, root.as_deref()),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

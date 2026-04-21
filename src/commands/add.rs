use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::config::{find_config, load_config, write_config, RepoEntry, RepoMode, CONFIG_FILENAME};

pub fn run(
    directory: &str,
    repo_url: &str,
    revision: &str,
    mode: &str,
    root: Option<&Path>,
    out: &mut dyn Write,
) -> Result<()> {
    let mode_enum = RepoMode::from_str_checked(mode)?;

    let (config_path, mut entries, storage_url) = match find_config(root) {
        Ok(cp) => {
            let config = load_config(&cp)?;
            (cp, config.repos, config.storage_url)
        }
        Err(_) => {
            let dir = root.unwrap_or(Path::new("."));
            let dir = std::fs::canonicalize(dir)?;
            (dir.join(CONFIG_FILENAME), Vec::new(), String::new())
        }
    };

    // Check for duplicate
    if entries.iter().any(|e| e.directory == directory) {
        anyhow::bail!(
            "'{}' is already declared in {}",
            directory,
            config_path.display()
        );
    }

    entries.push(RepoEntry {
        directory: directory.to_string(),
        repo_url: repo_url.to_string(),
        revision: revision.to_string(),
        mode: mode_enum,
    });

    write_config(&config_path, &entries, &storage_url)?;
    writeln!(
        out,
        "Added {} → {} @ {} [{}]",
        directory, repo_url, revision, mode
    )?;
    Ok(())
}

use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::config::{
    find_config, load_config, write_config, GitScaleConfig, RepoEntry, RepoMode, CONFIG_FILENAME,
};

pub fn run(
    directory: &str,
    repo_url: &str,
    revision: &str,
    mode: &str,
    root: Option<&Path>,
    out: &mut dyn Write,
) -> Result<()> {
    let mode_enum = RepoMode::from_str_checked(mode)?;

    // The whole config is carried through, not just the repo list: writing it
    // back replaces the file, so anything dropped here is deleted from disk.
    let (config_path, mut config) = match find_config(root) {
        Ok(cp) => {
            let config = load_config(&cp)?;
            (cp, config)
        }
        Err(_) => {
            let dir = root.unwrap_or(Path::new("."));
            let dir = std::fs::canonicalize(dir)?;
            (dir.join(CONFIG_FILENAME), GitScaleConfig::default())
        }
    };

    // Check for duplicate
    if config.repos.iter().any(|e| e.directory == directory) {
        anyhow::bail!(
            "'{}' is already declared in {}",
            directory,
            config_path.display()
        );
    }

    config.repos.push(RepoEntry {
        directory: directory.to_string(),
        repo_url: repo_url.to_string(),
        revision: revision.to_string(),
        mode: mode_enum,
        recursive: true,
    });

    write_config(&config_path, &config)?;
    writeln!(
        out,
        "Added {} → {} @ {} [{}]",
        directory, repo_url, revision, mode
    )?;
    Ok(())
}

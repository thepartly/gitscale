use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::config::{find_config, load_config, write_config};

pub fn run(directory: &str, root: Option<&Path>, out: &mut dyn Write) -> Result<()> {
    let config_path = find_config(root)?;
    let mut config = load_config(&config_path)?;
    let orig_len = config.repos.len();

    config.repos.retain(|e| e.directory != directory);

    if config.repos.len() == orig_len {
        anyhow::bail!(
            "'{}' is not declared in {}",
            directory,
            config_path.display()
        );
    }

    write_config(&config_path, &config)?;
    writeln!(out, "Removed {}", directory)?;
    Ok(())
}

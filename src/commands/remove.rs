use anyhow::Result;
use std::path::Path;

use crate::config::{find_config, load_config, write_config};

pub fn run(directory: &str, root: Option<&Path>) -> Result<()> {
    let config_path = find_config(root)?;
    let config = load_config(&config_path)?;
    let orig_len = config.repos.len();

    let remaining: Vec<_> = config
        .repos
        .into_iter()
        .filter(|e| e.directory != directory)
        .collect();

    if remaining.len() == orig_len {
        anyhow::bail!(
            "'{}' is not declared in {}",
            directory,
            config_path.display()
        );
    }

    write_config(&config_path, &remaining, &config.storage_url)?;
    println!("Removed {}", directory);
    Ok(())
}

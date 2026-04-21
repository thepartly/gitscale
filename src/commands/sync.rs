use anyhow::Result;
use std::path::Path;

use crate::config::{find_config, load_config};
use crate::hooks;

pub fn run(root: Option<&Path>, names: &[String], verbose: bool) -> Result<()> {
    crate::commands::clone::run(root, names, verbose)?;
    // pull runs its own post_sync hook, skip it here to avoid double-run
    crate::commands::pull::run_no_hooks(root, names, verbose)?;
    crate::commands::push::run(root, names, verbose)?;

    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;
    hooks::run_post_sync(&config.hooks, &config_root, verbose)?;
    Ok(())
}

use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::config::{find_config, load_config};
use crate::hooks;

pub fn run(root: Option<&Path>, names: &[String], verbose: bool, out: &mut dyn Write, err: &mut dyn Write) -> Result<()> {
    crate::commands::clone::run(root, names, verbose, out, err)?;
    // pull runs its own post_sync hook, skip it here to avoid double-run
    crate::commands::pull::run_no_hooks(root, names, verbose, out, err)?;
    crate::commands::push::run(root, names, verbose, out, err)?;

    let config_path = find_config(root)?;
    let config_root = config_path.parent().unwrap().to_path_buf();
    let config = load_config(&config_path)?;
    hooks::run_post_sync(&config.hooks, &config_root, verbose, out)?;
    Ok(())
}

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

use crate::config::Hooks;

pub fn run_post_sync(hooks: &Hooks, cwd: &Path, verbose: bool) -> Result<()> {
    let Some(cmd) = &hooks.post_sync else {
        return Ok(());
    };
    if cmd.is_empty() {
        return Ok(());
    }
    if verbose {
        println!("  hook  post_sync: {}", cmd);
    }
    let status = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("failed to run post_sync hook: {}", cmd))?;
    if !status.success() {
        anyhow::bail!(
            "post_sync hook failed (exit {}): {}",
            status.code().unwrap_or(-1),
            cmd
        );
    }
    Ok(())
}

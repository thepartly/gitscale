use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::Command;

use crate::config::Hooks;
use crate::trust::{Allowlist, Workspace};

/// Run the config's `post_sync` command.
///
/// The command comes out of the checked-out `.gitscale.toml`, so it is only as
/// trustworthy as the branch someone happens to have checked out. When gitscale
/// was started by an installed git hook — a `git clone` or `git checkout` that
/// the developer never aimed at gitscale at all — the shim passes down the
/// allowlist it was installed with, and a repository outside it runs nothing.
///
/// With no allowlist in the environment this is a `gitscale pull` or `sync` the
/// user typed, in a workspace they chose, and the hook runs.
pub fn run_post_sync(hooks: &Hooks, cwd: &Path, verbose: bool, out: &mut dyn Write) -> Result<()> {
    let Some(cmd) = &hooks.post_sync else {
        return Ok(());
    };
    if cmd.trim().is_empty() {
        return Ok(());
    }

    if let Some(allowlist) = Allowlist::from_env() {
        let workspace = Workspace::probe(cwd);
        match allowlist.matched_by(&workspace) {
            Some(pattern) => {
                if verbose {
                    writeln!(out, "  trust  post_sync allowed by '{}'", pattern)?;
                }
            }
            None => {
                // The command is echoed because the developer needs to see what
                // the branch tried to run, and defanged because they are about
                // to read it in a terminal.
                bail!(
                    "refusing to run the post_sync hook from an untrusted repository.\n\n\
                     The command was:\n\n    {}\n\n{}",
                    crate::trust::sanitize(cmd),
                    allowlist.explain(&workspace)
                );
            }
        }
    }

    if verbose {
        writeln!(out, "  hook  post_sync: {}", cmd)?;
    }
    let status = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("failed to run post_sync hook: {}", cmd))?;
    if !status.success() {
        bail!(
            "post_sync hook failed (exit {}): {}",
            status.code().unwrap_or(-1),
            cmd
        );
    }
    Ok(())
}

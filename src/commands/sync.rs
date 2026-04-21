use anyhow::Result;
use std::path::Path;

pub fn run(root: Option<&Path>, names: &[String], verbose: bool) -> Result<()> {
    crate::commands::clone::run(root, names, verbose)?;
    crate::commands::pull::run(root, names, verbose)?;
    crate::commands::push::run(root, names, verbose)?;
    Ok(())
}

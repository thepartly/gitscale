//! `git scale hash`: the source hash of the root or of checkouts — what each
//! one's own pipeline builds, as the tag its image carries. See
//! [`crate::hash`].

use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::artefact::Artefacts;
use crate::config::load_workspace;
use crate::hash::Workspace;
use crate::paths::{Here, Named};
use crate::resolve::Network;
use crate::store::Sources;

pub fn run(
    root: Option<&Path>,
    dirs: &[String],
    committed: bool,
    format: &str,
    verbose: bool,
    no_cache: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    let here = Here::new(root, &config_root)?;
    let sources = Sources::new(&config_root, no_cache)?;
    let artefacts = Artefacts::new(&config, &config_root, sources.images());
    let resolution = crate::resolve::workspace_with(
        &config,
        &config_root,
        Network::Offline.in_ci(),
        &sources,
        Some(&artefacts),
        verbose,
    )?;
    let ws = Workspace {
        root: &config_root,
        sources: &sources,
        artefacts: Some(&artefacts),
        resolution: &resolution,
        online: crate::git::is_ci(),
    };
    let asked: Vec<String> = if dirs.is_empty() {
        vec![".".to_string()]
    } else {
        dirs.to_vec()
    };
    let mut hashed = Vec::new();
    for arg in &asked {
        let dir = match here.name(&resolution, arg, true)? {
            Named::Root => None,
            Named::Slot(dir) => Some(dir),
        };
        hashed.push((arg, crate::hash::of(&ws, dir.as_deref(), committed)?));
    }
    if format == "json" {
        let rows: Vec<serde_json::Value> = hashed
            .iter()
            .map(|(arg, h)| {
                serde_json::json!({
                    "directory": arg,
                    "hash": h.hash,
                    "sources": h.sources.iter().map(|s| serde_json::json!({
                        "url": s.url,
                        "directory": s.directory,
                        "commit": s.commit,
                        "tree": s.tree,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        writeln!(out, "{}", serde_json::to_string_pretty(&rows)?)?;
    } else {
        for (arg, h) in &hashed {
            writeln!(out, "{}  {}", h.hash, arg)?;
        }
    }
    Ok(())
}

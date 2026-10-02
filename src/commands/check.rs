//! `gitscale check`: the merge gate. It fails while any checkout resolves
//! from a topic branch rather than a revision written in a config — what a
//! merge of this repository would ship is then not what its pipeline tested.
//! A merge request into a branch the root does not pin is not gated: that
//! branch follows topics itself. The check reads nothing but the resolution,
//! so it runs on a shallow checkout.

use anyhow::{bail, Result};
use std::io::Write;
use std::path::Path;

use crate::artefact::Artefacts;
use crate::config::{load_workspace, CONFIG_FILENAME};
use crate::store::Sources;

pub fn run(root: Option<&Path>, verbose: bool, no_cache: bool, out: &mut dyn Write) -> Result<()> {
    let (config, config_root) = load_workspace(root)?;
    let var = |name: &str| std::env::var(name).ok();
    if let Some(target) = crate::topic::Pipeline::target(&var) {
        let default = crate::topic::Pipeline::detect(&var)
            .and_then(|p| p.default)
            .or_else(|| crate::topic::default_branch(&config_root, true));
        let pinned = crate::topic::is_pinned(
            &target,
            config.develop.pinned.as_deref(),
            default.as_deref(),
        );
        if !pinned {
            writeln!(
                out,
                "ok: {} is not a pinned branch, so merging into it is not gated",
                target
            )?;
            return Ok(());
        }
    }
    let sources = Sources::new(&config_root, no_cache)?;
    let artefacts = Artefacts::new(&config, &config_root, sources.images());
    let resolution = crate::resolve::workspace(
        &config,
        &config_root,
        true,
        &sources,
        Some(&artefacts),
        verbose,
    )?;
    let what = if crate::git::is_ci() {
        "This pipeline"
    } else {
        "This workspace"
    };
    let mut blocks = Vec::new();
    for slot in resolution.topic_slots() {
        let Some(topic) = &slot.topic else {
            continue;
        };
        let pinned = match &topic.pin {
            Some(pin) => {
                let file = if pin.by == "root" {
                    CONFIG_FILENAME.to_string()
                } else {
                    format!("{}/{}", pin.by, CONFIG_FILENAME)
                };
                format!("{} pinned in {}", pin.revision, file)
            }
            None => "a revision pinned in a config".to_string(),
        };
        blocks.push(format!(
            "{dir} was taken from branch {branch}, not from {pinned}.\n  \
             {what} tested {dir} at {branch} ({sha}), so merging now would ship a pin that was \
             not tested.\n  \
             - {dir}'s change not merged yet: merge it first, then run gitscale upgrade here and \
             push.\n  \
             - already merged and pinned: delete branch {branch} in {url}, then rerun {rerun}.",
            dir = slot.directory,
            branch = topic.branch,
            pinned = pinned,
            what = what,
            sha = crate::git::short_sha(&topic.commit),
            url = slot.url,
            rerun = if crate::git::is_ci() {
                "this pipeline"
            } else {
                "gitscale check"
            },
        ));
    }
    if blocks.is_empty() {
        writeln!(out, "ok: every checkout resolves from a pinned revision")?;
        return Ok(());
    }
    bail!("{}", blocks.join("\n"))
}

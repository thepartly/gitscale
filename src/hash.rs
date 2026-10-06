//! The source hash: what a repository's own pipeline builds at its commit,
//! as one SHA-256 — its tree, and the tree of every repository its config
//! reaches, resolved as that pipeline resolves them. An image tagged with it
//! holds exactly those sources.
//!
//! The repository's config is resolved as if it were the root: on its topic
//! branch with that topic's branches, at its pin with pins only — whatever
//! else the workspace around it raised. So the hash its own CI takes and the
//! one taken of its checkout in another workspace agree. Git's object ids
//! are the input, so nothing is read but refs and commits, and a squash merge
//! that changes no file keeps the hash.

use anyhow::{anyhow, bail, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::artefact::Artefacts;
use crate::config::{parse_config, CONFIG_FILENAME};
use crate::resolution::{Checkouts, Engine, Resolution, Slot};
use crate::store::Sources;
use crate::stores::GitStores;

/// One repository a hash covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub url: String,
    /// Where this workspace checks the repository out: `None` for the root.
    pub directory: Option<String>,
    pub commit: String,
    pub tree: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hashed {
    pub hash: String,
    /// The repository itself first.
    pub sources: Vec<Source>,
}

/// The workspace a hash is taken in.
pub struct Workspace<'a> {
    pub root: &'a Path,
    pub sources: &'a Sources,
    pub artefacts: Option<&'a Artefacts>,
    pub resolution: &'a Resolution,
    /// Resolve against the remotes: what publishing does. Offline, as
    /// `git scale hash` is, outside CI.
    pub online: bool,
}

/// The hash of the repository at `dir` — a slot's directory, or the root for
/// `None`. Uncommitted changes in any of its sources fail it, unless
/// `committed`: then each source is taken at its commit, changes left out.
pub fn of(ws: &Workspace, dir: Option<&str>, committed: bool) -> Result<Hashed> {
    let label = dir.unwrap_or(".");
    let own = own(ws, dir, committed)?;
    let mut sources = vec![Source {
        url: own.url.clone(),
        directory: dir.map(str::to_string),
        tree: own.tree,
        commit: own.commit,
    }];

    if let Some(text) = own.config {
        let config = parse_config(
            &text,
            &PathBuf::from(format!("{}/{}", label, CONFIG_FILENAME)),
        )?;
        let stores = GitStores::new(ws.root, ws.sources, ws.online, ws.artefacts, false);
        let resolved = Engine::new(&config, Some(&own.url), &stores, &Committed)
            .with_topic(own.topic)
            .resolve()?;
        for slot in &resolved.slots {
            let commit = slot.commit.clone().ok_or_else(|| {
                anyhow!(
                    "{}: {} does not resolve{}",
                    label,
                    slot.directory,
                    slot.unresolved
                        .as_deref()
                        .map(|why| format!(": {}", why))
                        .unwrap_or_default()
                )
            })?;
            let checkout = checkout_at(ws, &slot.url, &commit);
            if let (Some(at), false) = (&checkout, committed) {
                let planted = ws.resolution.planted_in(at);
                if crate::git::uncommitted(&ws.root.join(at), &planted).is_some() {
                    bail!(uncommitted(at));
                }
            }
            sources.push(Source {
                url: slot.url.clone(),
                tree: tree_of(ws, slot, &commit, checkout.as_deref())?,
                directory: checkout.or_else(|| directory_of(ws, &slot.url)),
                commit,
            });
        }
    }

    let (own, rest) = sources.split_first().expect("the repository itself");
    let line = |s: &Source| format!("{} {}\n", crate::urls::normalize(&s.url), s.tree);
    let mut input = line(own);
    let others: BTreeSet<String> = rest.iter().map(line).collect();
    input.extend(others);
    let hash = hex::encode(Sha256::digest(input.as_bytes()));
    Ok(Hashed { hash, sources })
}

/// What a hash starts from: the repository itself.
struct Own {
    url: String,
    commit: String,
    tree: String,
    /// Its `.gitscale.toml` at the commit, when it has one.
    config: Option<String>,
    /// The topic its own pipeline would be on: its branch, when that is not
    /// one its config pins.
    topic: Option<String>,
}

/// The repository at `dir`: a git checkout — the root's, or a slot's — read
/// at its HEAD, or a slot taken as an artefact, read at its commit from its
/// store.
fn own(ws: &Workspace, dir: Option<&str>, committed: bool) -> Result<Own> {
    let label = dir.unwrap_or(".");
    let repo = dir.map_or_else(|| ws.root.to_path_buf(), |d| ws.root.join(d));
    if dir.is_none() || crate::git::is_checkout(&repo) {
        let commit = crate::git::resolve_ref(&repo, "HEAD")
            .ok_or_else(|| anyhow!("{} has no commit to hash", label))?;
        if !committed {
            let changed = match dir {
                Some(dir) => crate::git::uncommitted(&repo, &ws.resolution.planted_in(dir)),
                None => ws.resolution.root_changes(ws.root),
            };
            if changed.is_some() {
                bail!(uncommitted(label));
            }
        }
        let config = crate::config::committed_at(&repo, "HEAD");
        let topic = config
            .as_deref()
            .and_then(|text| parse_config(text, &repo.join(CONFIG_FILENAME)).ok())
            .and_then(|config| {
                crate::topic::root(&config, &repo, false)
                    .topic()
                    .map(str::to_string)
            });
        return Ok(Own {
            url: crate::git::origin_url(&repo).ok_or_else(|| {
                anyhow!(
                    "{} has no origin remote, and its URL is part of the hash",
                    label
                )
            })?,
            tree: tree(&repo, &commit, label)?,
            commit,
            config,
            topic,
        });
    }
    let slot = ws
        .resolution
        .find(label)
        .ok_or_else(|| anyhow!("{} is not a checkout of this workspace", label))?;
    let commit = slot
        .commit
        .clone()
        .ok_or_else(|| anyhow!("{} does not resolve", label))?;
    let (tree, config) = match holding(ws, &slot.url, &commit) {
        Some(store) => (
            tree(&store, &commit, label)?,
            crate::config::committed_at(&store, &commit),
        ),
        None => {
            let artefacts = ws.artefacts.ok_or_else(|| not_here(&slot.url, &commit))?;
            let revision = slot.entry().revision;
            let tag = crate::artefact::image_tag(&revision)
                .ok_or_else(|| not_here(&slot.url, &commit))?;
            (
                tree_of(ws, slot, &commit, None)?,
                artefacts.config(&slot.url, tag, ws.online)?,
            )
        }
    };
    Ok(Own {
        url: slot.url.clone(),
        tree,
        config,
        topic: slot.topic.as_ref().map(|t| t.branch.clone()),
        commit,
    })
}

/// The tree of `slot` at `commit`: from the workspace's checkout of it at
/// that commit, else from its store — or, for a release taken as an
/// artefact, from what the release's image records.
fn tree_of(ws: &Workspace, slot: &Slot, commit: &str, checkout: Option<&str>) -> Result<String> {
    if let Some(at) = checkout {
        return tree(&ws.root.join(at), commit, at);
    }
    if let Some(store) = holding(ws, &slot.url, commit) {
        return tree(&store, commit, &slot.url);
    }
    let revision = slot.entry().revision;
    let Some(tag) = crate::artefact::image_tag(&revision) else {
        return Err(not_here(&slot.url, commit));
    };
    ws.artefacts
        .map(|a| a.tree(&slot.url, tag, ws.online))
        .transpose()?
        .flatten()
        .ok_or_else(|| {
            anyhow!(
                "the image of {} at {} records no tree; it was published before trees were",
                slot.url,
                tag
            )
        })
}

/// The root's store for `url`, when it holds `commit`.
fn holding(ws: &Workspace, url: &str, commit: &str) -> Option<PathBuf> {
    let store = ws
        .sources
        .stores
        .as_ref()?
        .repo_path(&crate::ci::remote_url(url));
    crate::git::ref_exists(&store, &format!("{}^{{commit}}", commit)).then_some(store)
}

fn uncommitted(dir: &str) -> String {
    format!(
        "{} has uncommitted changes; commit them, or --committed to hash its commit",
        dir
    )
}

fn not_here(url: &str, commit: &str) -> anyhow::Error {
    anyhow!(
        "{} at {} is not on this machine; run git scale fetch",
        url,
        crate::git::short_sha(commit)
    )
}

/// The tree of `commit` in the repository at `repo`.
fn tree(repo: &Path, commit: &str, what: &str) -> Result<String> {
    crate::git::resolve_ref(repo, &format!("{}^{{tree}}", commit)).ok_or_else(|| {
        anyhow!(
            "{}: {} is not in its repository; run git scale fetch",
            what,
            crate::git::short_sha(commit)
        )
    })
}

/// The workspace's checkout of `url` sitting at `commit`, if there is one.
fn checkout_at(ws: &Workspace, url: &str, commit: &str) -> Option<String> {
    ws.resolution
        .slots_of(url)
        .map(|s| &s.directory)
        .find(|dir| {
            let dest: PathBuf = ws.root.join(dir);
            crate::git::is_checkout(&dest)
                && crate::git::resolve_ref(&dest, "HEAD").as_deref() == Some(commit)
        })
        .cloned()
}

/// Where the workspace checks `url` out, at whatever commit.
fn directory_of(ws: &Workspace, url: &str) -> Option<String> {
    ws.resolution
        .slots_of(url)
        .next()
        .map(|s| s.directory.clone())
}

/// Every config read from its repository at the commit asked for, never from
/// a working tree: a hash is of what is committed.
struct Committed;

impl Checkouts for Committed {
    fn config_if_at(&self, _: &str, _: &str) -> Option<Option<String>> {
        None
    }

    fn head(&self, _: &str) -> Option<String> {
        None
    }
}

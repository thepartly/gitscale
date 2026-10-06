//! Promotion: whether a topic slot's change has reached a release, and the
//! order the topic's repositories merge in.
//!
//! No merge strategy is assumed. A squash or a rebase rewrites commits, so
//! history cannot tell whether a branch was merged; content can. A topic
//! branch is in a tag when merging it into the tag would change nothing —
//! `git merge-tree` writes the result, and it is the tag's own tree. Nor is a
//! branch flow assumed: only tags are asked, whichever branch they were cut
//! on.
//!
//! This needs history, so it runs in the root's store for the slot's
//! repository, which every checkout of it shares. Never in CI: a pipeline
//! keeps no stores, and nothing a pipeline does depends on it.

use anyhow::{bail, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::artefact::Artefacts;
use crate::config::{parse_dependency_config, GitScaleConfig, CONFIG_FILENAME};
use crate::resolution::{Pin, Resolution, Slot};
use crate::store::Stores;
use crate::version::{self, Version};

/// The tags among `tags` a pin at `pin` may move to by their names alone,
/// newest first: versions of its own kind, of its own major unless
/// `cross_major`, and pre-releases only when the pin already is one — the
/// rule npm and Cargo follow.
pub fn by_name<'a>(
    tags: impl IntoIterator<Item = &'a str>,
    pin: &Version,
    cross_major: bool,
) -> Vec<(String, Version)> {
    let mut found: Vec<(String, Version)> = tags
        .into_iter()
        .filter_map(|tag| Some((tag.to_string(), version::parse(tag)?)))
        .filter(|(_, v)| pin.compare(v).is_some())
        .filter(|(_, v)| cross_major || v.class() == pin.class())
        .filter(|(_, v)| !v.is_prerelease() || pin.is_prerelease())
        .collect();
    found.sort_by(|(_, a), (_, b)| b.compare(a).unwrap_or(std::cmp::Ordering::Equal));
    found
}

/// The releases a pin may move to, newest first: [`by_name`], among the tags
/// reachable from the repository's pinned branches, and whose history holds
/// the pin's commit — a line split off before the pin would lose what the
/// pin had — unless `cross_major`, since a new major is often cut on a line
/// of its own.
pub fn candidates(
    store: &Path,
    slot: &Slot,
    pin: &Pin,
    pinned: &Version,
    cross_major: bool,
) -> Result<Vec<(String, Version)>> {
    let lines = |args: &[&str]| -> BTreeSet<String> {
        crate::git::query(store, args)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    };
    let mut tags = if cross_major {
        lines(&["tag", "--list"])
    } else {
        lines(&["tag", "--contains", &pin.commit])
    };
    let remote: Vec<String> = lines(&[
        "for-each-ref",
        "--format=%(refname:strip=3)",
        "refs/remotes/origin",
    ])
    .into_iter()
    .filter(|b| b != "HEAD")
    .collect();
    let merged: BTreeSet<String> = pinned_branches(store, slot, pin, &remote)?
        .iter()
        .flat_map(|b| lines(&["tag", "--merged", &format!("refs/remotes/origin/{}", b)]))
        .collect();
    tags.retain(|t| merged.contains(t));
    Ok(by_name(
        tags.iter().map(String::as_str),
        pinned,
        cross_major,
    ))
}

/// The branches among `remote` that `slot`'s repository pins, where its
/// releases are: those its `[branches] pinned` names, as its default branch
/// has them — the policy as it is now, though the pin may predate it — else
/// its default branch.
fn pinned_branches(store: &Path, slot: &Slot, pin: &Pin, remote: &[String]) -> Result<Vec<String>> {
    let head = crate::git::query(
        store,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    );
    let default = head
        .as_deref()
        .and_then(|h| h.strip_prefix("origin/"))
        .map(str::to_string);
    let at = if head.is_some() {
        "refs/remotes/origin/HEAD"
    } else {
        pin.commit.as_str()
    };
    let path = PathBuf::from(format!("{}/{}", slot.url, CONFIG_FILENAME));
    let pinned = crate::config::committed_at(store, at)
        .map(|text| parse_dependency_config(&text, &path))
        .transpose()?
        .and_then(|config| config.branches.pinned);
    let found: Vec<String> = remote
        .iter()
        .filter(|b| crate::topic::is_pinned(b, pinned.as_deref(), default.as_deref()))
        .cloned()
        .collect();
    if found.is_empty() {
        bail!(
            "no branch of {} is pinned{}, so it has no releases",
            slot.url,
            pinned
                .map(|p| format!(": [branches] pinned = {}", p.join(", ")))
                .unwrap_or_default()
        );
    }
    Ok(found)
}

/// What a raise of a slot moves its requests to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The pin is no version, so there is nothing to look past.
    NotAVersion,
    /// No release of the pin's kind and major: the pin is given.
    NoRelease(String),
    /// The newest release, and the pin resolution selected — equal when one
    /// request already asks for the newest and others for less.
    Release { pin: String, newest: String },
}

impl Target {
    /// What a raise of `dir` moves to, or why it moves to nothing.
    pub fn release(&self, dir: &str) -> std::result::Result<(&str, &str), String> {
        match self {
            Target::NotAVersion => Err(format!(
                "{} is not pinned to a version; nothing to raise from",
                dir
            )),
            Target::NoRelease(pin) => Err(format!("{} has no release to raise {} to", dir, pin)),
            Target::Release { pin, newest } => Ok((pin, newest)),
        }
    }
}

/// What a raise of `slot` moves its requests to: the newest of its
/// [`candidates`] in the root's store for its repository, fetched. Without
/// access to its sources, the newest of its registry's version tags: each is
/// cut after the last on the branches releases are cut on, so it holds them.
pub fn target(
    stores: &Stores,
    artefacts: &Artefacts,
    slot: &Slot,
    cross_major: bool,
) -> Result<Target> {
    let Some(pin) = slot.pin() else {
        return Ok(Target::NotAVersion);
    };
    let Some(pinned) = version::parse(&pin.revision) else {
        return Ok(Target::NotAVersion);
    };
    let found = if slot.sourceless {
        let released = artefacts.released(&slot.url)?;
        by_name(released.keys().map(String::as_str), &pinned, cross_major)
    } else {
        let store = stores.update(&crate::ci::remote_url(&slot.url))?;
        candidates(&store, slot, &pin, &pinned, cross_major)?
    };
    let newest = found.into_iter().next();
    Ok(match newest {
        Some((tag, found)) if pinned.compare(&found) != Some(std::cmp::Ordering::Greater) => {
            Target::Release {
                pin: pin.revision,
                newest: tag,
            }
        }
        _ => Target::NoRelease(pin.revision),
    })
}

/// Whether `revision` is a version below `tag`: `None` when the two are not
/// versions of one kind, so nothing can be said.
pub fn below(revision: &str, tag: &str) -> Option<bool> {
    let (have, want) = (version::parse(revision)?, version::parse(tag)?);
    have.compare(&want)
        .map(|order| order == std::cmp::Ordering::Less)
}

/// Whether the commit `tip` adds anything to `tag`, in the repository at
/// `repo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Containment {
    /// Merging `tip` into the tag changes nothing: its change is in.
    Contained,
    /// It would change something.
    Adds,
    /// The merge conflicts: a later commit rewrote the same lines, and
    /// content cannot tell.
    Conflicts,
}

pub fn containment(repo: &Path, tag: &str, tip: &str) -> Result<Containment> {
    let tag_tree = crate::git::resolve_ref(repo, &format!("{}^{{tree}}", tag))
        .ok_or_else(|| anyhow::anyhow!("{} is not in the checkout", crate::git::short_sha(tag)))?;
    let merged = crate::git::run_git(
        &["merge-tree", "--write-tree", "--no-messages", tag, tip],
        Some(repo),
        false,
    )?;
    match merged.status.code() {
        Some(0) => {
            let tree = String::from_utf8_lossy(&merged.stdout)
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            Ok(if tree == tag_tree {
                Containment::Contained
            } else {
                Containment::Adds
            })
        }
        Some(1) => Ok(Containment::Conflicts),
        _ => bail!(
            "git merge-tree failed: {}",
            String::from_utf8_lossy(&merged.stderr).trim()
        ),
    }
}

/// Where a topic slot's change stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// The branch adds nothing to what the configs pin yet.
    Unchanged,
    /// No release holds the change.
    NoTag,
    /// No release holds the pin's commit — its tag was moved, or it was
    /// cut on no release branch — so none can be looked for past it.
    NoRelease(String),
    /// The tag holds the change, but the tag's commit has no image — for
    /// a repository that publishes artefacts.
    NoImage(String),
    /// The tag holds the change: released, waiting for `upgrade` to
    /// promote it.
    Released(String),
    /// Merging conflicts, so content cannot tell.
    CannotTell(String),
    /// Uncommitted work, or commits nobody has: what could be in a tag is
    /// not what is here.
    Held(String),
    /// Nothing says which release to look for: the slot is pinned to no
    /// version, or its checkout has no history to ask.
    Unknown(String),
}

impl State {
    /// As `status` and `upgrade` say it.
    pub fn describe(&self) -> String {
        match self {
            State::Unchanged => "no change yet".to_string(),
            State::NoTag => "no tag".to_string(),
            State::NoRelease(pin) => format!("no release contains {}", pin),
            State::NoImage(tag) => format!("tagged {}, no image", tag),
            State::Released(tag) => format!("released as {}", tag),
            State::CannotTell(why) => format!("cannot tell ({})", why),
            State::Held(why) => format!("held ({})", why),
            State::Unknown(why) => why.clone(),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            State::Unchanged => "unchanged",
            State::NoTag => "no-tag",
            State::NoRelease(_) => "no-release",
            State::NoImage(_) => "no-image",
            State::Released(_) => "released",
            State::CannotTell(_) => "cannot-tell",
            State::Held(_) => "held",
            State::Unknown(_) => "unknown",
        }
    }

    pub fn is_released(&self) -> bool {
        matches!(self, State::Released(_))
    }
}

/// Asks whether a release has an artefact image.
pub type HasImage<'a> = &'a dyn Fn(&str) -> Result<bool>;

/// Where the topic slot `slot` stands, read from `store`, the root's store
/// for its repository. `has_image` answers whether a release has an
/// artefact image, for a repository that publishes them; `None` skips that question,
/// for a command that stays offline.
pub fn assess(
    config_root: &Path,
    store: &Path,
    slot: &Slot,
    planted: &[String],
    has_image: Option<HasImage>,
) -> State {
    let Some(topic) = &slot.topic else {
        return State::Unknown("not on the topic".to_string());
    };
    let Some(pin) = topic.pin.as_ref() else {
        return State::Unknown("pinned to no version".to_string());
    };
    let Some(pinned) = version::parse(&pin.revision) else {
        return State::Unknown(format!("{} is not a version to look past", pin.revision));
    };
    if !crate::store::is_repository(store) {
        return State::Unknown("not fetched yet".to_string());
    }
    // Uncommitted work is not in any tag. Commits nobody else has are fine:
    // once the tag holds their content, they are what a squash left behind.
    let dest = config_root.join(&slot.directory);
    let on_branch = crate::git::is_checkout(&dest)
        && crate::git::current_branch(&dest).as_deref() == Some(topic.branch.as_str());
    if on_branch {
        if let Some(changed) = crate::git::uncommitted(&dest, planted) {
            return State::Held(format!(
                "{} uncommitted",
                crate::cache::plural(changed, "change", "changes")
            ));
        }
    }
    let tip = topic.commit.clone();
    // Nothing on the branch the pin does not already hold: no change to
    // release.
    if matches!(
        containment(store, &pin.commit, &tip),
        Ok(Containment::Contained)
    ) {
        return State::Unchanged;
    }
    let candidates = match candidates(store, slot, pin, &pinned, false) {
        Ok(found) if found.is_empty() => return State::NoRelease(pin.revision.clone()),
        Ok(found) => found,
        Err(e) => return State::CannotTell(format!("{:#}", e)),
    };
    // The newest release that holds the change: a newer one cut beside it —
    // a hotfix from the pin — may not.
    let mut holding = None;
    for (tag, found) in &candidates {
        if pinned.compare(found) != Some(std::cmp::Ordering::Less) {
            break;
        }
        match containment(store, tag, &tip) {
            Ok(Containment::Contained) => {
                holding = Some(tag.clone());
                break;
            }
            Ok(Containment::Adds) => continue,
            Ok(Containment::Conflicts) => {
                return State::CannotTell(format!(
                    "a later commit in {} rewrote the same lines",
                    tag
                ))
            }
            Err(e) => return State::CannotTell(format!("{:#}", e)),
        }
    }
    let Some(tag) = holding else {
        return State::NoTag;
    };
    // A repository that publishes artefacts is released once its image is
    // there too, whatever form anyone takes it in.
    if let Some(has_image) = has_image {
        if publishes_artefacts(store, &tag) {
            match has_image(&tag) {
                Ok(true) => {}
                Ok(false) => return State::NoImage(tag),
                Err(e) => return State::CannotTell(format!("{:#}", e)),
            }
        }
    }
    State::Released(tag)
}

/// Whether the repository in `store` publishes artefacts at `revision`: its
/// config there has an `[artefact]` table.
fn publishes_artefacts(store: &Path, revision: &str) -> bool {
    crate::config::committed_at(store, revision)
        .and_then(|text| text.parse::<toml::Table>().ok())
        .is_some_and(|config| config.contains_key("artefact"))
}

/// The `.gitscale.toml` of a checkout as it is on disk, edits included.
pub fn working_config(dir: &Path) -> Option<GitScaleConfig> {
    let path = dir.join(CONFIG_FILENAME);
    let text = std::fs::read_to_string(&path).ok()?;
    parse_dependency_config(&text, &path).ok()
}

/// What each of the topic's repositories asks for among the others: by
/// directory (`.` for the root), the topic slots its config requests.
pub fn topic_requests(
    config: &GitScaleConfig,
    config_root: &Path,
    resolution: &Resolution,
) -> BTreeMap<String, BTreeSet<String>> {
    let topic = resolution.topic_slots();
    let by_url: BTreeMap<String, &str> = topic
        .iter()
        .map(|s| (crate::urls::normalize(&s.url), s.directory.as_str()))
        .collect();
    let wanted = |config: &GitScaleConfig| -> BTreeSet<String> {
        config
            .repos
            .iter()
            .filter_map(|e| by_url.get(&crate::urls::normalize(&e.repo_url)))
            .map(|d| d.to_string())
            .collect()
    };
    let mut requests = BTreeMap::new();
    requests.insert(".".to_string(), wanted(config));
    for slot in &topic {
        let own = working_config(&config_root.join(&slot.directory))
            .map(|c| wanted(&c))
            .unwrap_or_default();
        requests.insert(slot.directory.clone(), own);
    }
    requests
}

/// Which of the topic's repositories may merge next: those not promoted
/// whose topic dependencies, all the way down, are. The root merges last.
pub fn next_to_merge(
    requests: &BTreeMap<String, BTreeSet<String>>,
    promoted: &BTreeSet<String>,
) -> Vec<String> {
    let waits = |dir: &str| -> bool {
        let mut stack: Vec<&str> = requests
            .get(dir)
            .map(|r| r.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let mut seen = BTreeSet::new();
        while let Some(next) = stack.pop() {
            if !seen.insert(next) {
                continue;
            }
            if !promoted.contains(next) {
                return true;
            }
            if let Some(more) = requests.get(next) {
                stack.extend(more.iter().map(String::as_str));
            }
        }
        false
    };
    let ready: Vec<String> = requests
        .keys()
        .filter(|d| d.as_str() != "." && !promoted.contains(*d) && !waits(d))
        .cloned()
        .collect();
    if ready.is_empty() && requests.keys().all(|d| d == "." || promoted.contains(d)) {
        return vec![".".to_string()];
    }
    ready
}

/// The topic slots `dir` waits on directly: requested and not promoted.
pub fn waits_on(
    requests: &BTreeMap<String, BTreeSet<String>>,
    promoted: &BTreeSet<String>,
    dir: &str,
) -> Vec<String> {
    requests
        .get(dir)
        .map(|r| {
            r.iter()
                .filter(|d| !promoted.contains(*d))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(tag: &str) -> Version {
        version::parse(tag).unwrap()
    }

    /// The newest by name alone.
    fn newest<'a>(
        tags: impl IntoIterator<Item = &'a str>,
        pin: &Version,
        cross: bool,
    ) -> Option<(String, Version)> {
        by_name(tags, pin, cross).into_iter().next()
    }

    #[test]
    fn the_newest_release_of_the_pins_kind_and_major() {
        let tags = [
            "v1-2026.09.28",
            "v1-2026.10.01",
            "v1-2026.10.02-rc1",
            "v2-2026.12.01",
            "api-v1-2026.12.01",
            "v1.9.0",
        ];
        assert_eq!(
            newest(tags, &v("v1-2026.09.28"), false).unwrap().0,
            "v1-2026.10.01"
        );
        assert_eq!(
            newest(tags, &v("v1-2026.09.28"), true).unwrap().0,
            "v2-2026.12.01"
        );
        let semver = ["v1.2.0", "v1.4.0", "v2.0.0", "v1.5.0-rc.1"];
        assert_eq!(newest(semver, &v("v1.2.0"), false).unwrap().0, "v1.4.0");
        assert_eq!(newest(semver, &v("v1.2.0"), true).unwrap().0, "v2.0.0");
    }

    #[test]
    fn a_pre_release_only_for_a_pin_that_is_one() {
        let tags = ["v1-2026.10.01-rc1", "v1-2026.10.01-rc2", "v1-2026.09.30"];
        assert_eq!(
            newest(tags, &v("v1-2026.09.30"), false).unwrap().0,
            "v1-2026.09.30"
        );
        assert_eq!(
            newest(tags, &v("v1-2026.09.30-rc1"), false).unwrap().0,
            "v1-2026.10.01-rc2"
        );
    }

    #[test]
    fn the_merge_order_follows_the_topic_requests() {
        let mut requests = BTreeMap::new();
        requests.insert(".".to_string(), BTreeSet::from(["b".to_string()]));
        requests.insert("b".to_string(), BTreeSet::from(["d".to_string()]));
        requests.insert("d".to_string(), BTreeSet::new());
        requests.insert("c".to_string(), BTreeSet::new());
        let none = BTreeSet::new();
        assert_eq!(next_to_merge(&requests, &none), vec!["c", "d"]);
        let d = BTreeSet::from(["d".to_string(), "c".to_string()]);
        assert_eq!(next_to_merge(&requests, &d), vec!["b"]);
        assert_eq!(waits_on(&requests, &none, "b"), vec!["d"]);
        let all = BTreeSet::from(["d".to_string(), "c".to_string(), "b".to_string()]);
        assert_eq!(next_to_merge(&requests, &all), vec!["."]);
    }
}

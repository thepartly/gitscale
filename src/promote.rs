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
use std::path::Path;

use crate::config::{parse_dependency_config, GitScaleConfig, CONFIG_FILENAME};
use crate::resolution::{Resolution, Slot};
use crate::version::{self, Version};

/// The newest tag among `tags` that a pin at `pin` may move to: a version of
/// its own stream and kind, of its own major unless `cross_major`, and a
/// pre-release only when the pin already is one — the rule npm and Cargo
/// follow.
pub fn newest<'a>(
    tags: impl IntoIterator<Item = &'a str>,
    pin: &Version,
    cross_major: bool,
) -> Option<(String, Version)> {
    let mut best: Option<(String, Version)> = None;
    for tag in tags {
        let Some(found) = version::parse(tag) else {
            continue;
        };
        if pin.compare(&found).is_none() {
            continue;
        }
        if !cross_major && found.class() != pin.class() {
            continue;
        }
        if found.is_prerelease() && !pin.is_prerelease() {
            continue;
        }
        let better = match &best {
            None => true,
            Some((_, current)) => current.compare(&found) == Some(std::cmp::Ordering::Less),
        };
        if better {
            best = Some((tag.to_string(), found));
        }
    }
    best
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
    /// The newest release does not hold the change yet: not merged, or
    /// merged with its tagging pipeline still to run.
    NotTagged,
    /// The tag holds the change, but the tag's commit has no image yet —
    /// for a slot consumed as an artefact.
    NoImage(String),
    /// The tag holds the change: ready for `upgrade`.
    Promoted(String),
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
            State::NotTagged => "not tagged yet".to_string(),
            State::NoImage(tag) => format!("tagged {}, no image yet", tag),
            State::Promoted(tag) => format!("promoted → {}", tag),
            State::CannotTell(why) => format!("cannot tell ({})", why),
            State::Held(why) => format!("held ({})", why),
            State::Unknown(why) => why.clone(),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            State::Unchanged => "unchanged",
            State::NotTagged => "not-tagged",
            State::NoImage(_) => "no-image",
            State::Promoted(_) => "promoted",
            State::CannotTell(_) => "cannot-tell",
            State::Held(_) => "held",
            State::Unknown(_) => "unknown",
        }
    }

    pub fn is_promoted(&self) -> bool {
        matches!(self, State::Promoted(_))
    }
}

/// Asks whether a commit has an artefact image.
pub type HasImage<'a> = &'a dyn Fn(&str) -> Result<bool>;

/// Where the topic slot `slot` stands, read from `store`, the root's store
/// for its repository. `has_image` answers whether a commit has an artefact
/// image, for a slot consumed as one; `None` skips that question, for a
/// command that stays offline.
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
    let tags = crate::git::query(store, &["tag", "--list"]).unwrap_or_default();
    let Some((tag, found)) = newest(tags.lines(), &pinned, false) else {
        return State::NotTagged;
    };
    if pinned.compare(&found) != Some(std::cmp::Ordering::Less) {
        return State::NotTagged;
    }
    match containment(store, &tag, &tip) {
        Ok(Containment::Contained) => {}
        Ok(Containment::Adds) => return State::NotTagged,
        Ok(Containment::Conflicts) => {
            return State::CannotTell(format!("a later commit in {} rewrote the same lines", tag))
        }
        Err(e) => return State::CannotTell(format!("{:#}", e)),
    }
    if let Some(has_image) = has_image {
        if slot.artefact.is_some() {
            let commit =
                crate::git::resolve_ref(store, &format!("{}^{{commit}}", tag)).unwrap_or_default();
            match has_image(&commit) {
                Ok(true) => {}
                Ok(false) => return State::NoImage(tag),
                Err(e) => return State::CannotTell(format!("{:#}", e)),
            }
        }
    }
    State::Promoted(tag)
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

    #[test]
    fn the_newest_release_of_the_pins_stream_and_major() {
        let tags = [
            "v2026.09.28",
            "v2026.10.01",
            "v2026.10.02-rc1",
            "api-2026.12.01",
            "v1.9.0",
        ];
        assert_eq!(
            newest(tags, &v("v2026.09.28"), false).unwrap().0,
            "v2026.10.01"
        );
        let semver = ["v1.2.0", "v1.4.0", "v2.0.0", "v1.5.0-rc.1"];
        assert_eq!(newest(semver, &v("v1.2.0"), false).unwrap().0, "v1.4.0");
        assert_eq!(newest(semver, &v("v1.2.0"), true).unwrap().0, "v2.0.0");
    }

    #[test]
    fn a_pre_release_only_for_a_pin_that_is_one() {
        let tags = ["v2026.10.01-rc1", "v2026.10.01-rc2", "v2026.09.30"];
        assert_eq!(
            newest(tags, &v("v2026.09.30"), false).unwrap().0,
            "v2026.09.30"
        );
        assert_eq!(
            newest(tags, &v("v2026.09.30-rc1"), false).unwrap().0,
            "v2026.10.01-rc2"
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

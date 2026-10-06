//! How each checkout arrives: its sources, or the artefact of its release.
//! A config says what a repository
//! depends on; the form is this workspace's choice, per dependency, recorded
//! by `git scale prefer` in the root's git common dir — shared by every
//! worktree of the root, keyed by normalised URL, so it covers every major of
//! the repository wherever it is checked out.
//!
//! Which form a checkout gets, the first that applies:
//!
//! 1. on the workspace's topic — joined here, or a branch of the topic
//!    followed: its sources;
//! 2. its sources cannot be read: its artefact;
//! 3. its preference;
//! 4. its sources.
//!
//! An artefact is only ever a release's: the image its version tag names.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// What a checkout's directory holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Form {
    /// A worktree of the repository's store.
    Source,
    /// The image of its release, and nothing else, read-only.
    Artefact,
}

impl fmt::Display for Form {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Form::Source => "source",
            Form::Artefact => "artefact",
        })
    }
}

/// Why a checkout has its form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Topic,
    NoAccess,
    Preferred,
    Default,
}

impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Reason::Topic => "topic",
            Reason::NoAccess => "no-access",
            Reason::Preferred => "preferred",
            Reason::Default => "default",
        }
    }
}

/// The form a checkout of `dir` has on disk now: `None` when there is none.
pub fn on_disk(config_root: &Path, dir: &str) -> Option<Form> {
    let dest = config_root.join(dir);
    if crate::git::is_checkout(&dest) {
        return Some(Form::Source);
    }
    crate::artefact::installed(config_root, dir).map(|_| Form::Artefact)
}

/// The workspace's preferences: by normalised URL, the form other than
/// `source` each repository is to take.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default)]
    repos: BTreeMap<String, Form>,
}

impl Prefs {
    /// The preferences of the workspace at `config_root`; none when it has
    /// recorded none.
    pub fn load(config_root: &Path) -> Result<Prefs> {
        let path = path(config_root);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("{}: unreadable", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Prefs::default()),
            Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
        }
    }

    pub fn save(&self, config_root: &Path) -> Result<()> {
        let path = path(config_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let partial = path.with_extension("partial");
        std::fs::write(&partial, toml::to_string(self)?)
            .with_context(|| format!("cannot write {}", partial.display()))?;
        std::fs::rename(&partial, &path).with_context(|| format!("cannot write {}", path.display()))
    }

    /// The form preferred for `url`, other than `source`.
    pub fn get(&self, url: &str) -> Option<Form> {
        self.repos.get(&crate::urls::normalize(url)).copied()
    }

    /// Prefer `form` for `url`; `source`, the default, removes the preference.
    pub fn set(&mut self, url: &str, form: Form) {
        let key = crate::urls::normalize(url);
        match form {
            Form::Source => {
                self.repos.remove(&key);
            }
            form => {
                self.repos.insert(key, form);
            }
        }
    }

    /// Every preference, by normalised URL.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Form)> {
        self.repos.iter().map(|(url, form)| (url.as_str(), *form))
    }
}

/// `gitscale/prefer.toml` in the root's git common dir.
fn path(config_root: &Path) -> PathBuf {
    let common = crate::git::query(
        config_root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map(PathBuf::from)
    .unwrap_or_else(|| config_root.join(".git"));
    common.join("gitscale").join("prefer.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_is_the_default_and_removes_a_preference() {
        let mut prefs = Prefs::default();
        prefs.set("git@github.com:acme/core.git", Form::Artefact);
        assert_eq!(
            prefs.get("https://github.com/acme/core"),
            Some(Form::Artefact)
        );
        prefs.set("https://github.com/acme/core.git", Form::Source);
        assert_eq!(prefs.get("https://github.com/acme/core"), None);
        assert_eq!(prefs, Prefs::default());
    }

    #[test]
    fn it_reads_back_what_it_writes() {
        let mut prefs = Prefs::default();
        prefs.set("https://github.com/acme/sdk.git", Form::Artefact);
        let text = toml::to_string(&prefs).unwrap();
        assert!(
            text.contains("[repos]") && text.contains("= \"artefact\""),
            "{}",
            text
        );
        assert_eq!(toml::from_str::<Prefs>(&text).unwrap(), prefs);
    }
}

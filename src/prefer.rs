//! How each checkout arrives: its sources, or the artefact of its release —
//! and whether a lazy one is there at all. A config says what a repository
//! depends on, and may commit `lazy = true` for it; the form, and whether to
//! keep a repository lazy or not, is this workspace's choice, per dependency,
//! recorded by `git scale prefer` in the root's git common dir — shared by
//! every worktree of the root, keyed by normalised URL, so it covers every
//! major of the repository wherever it is checked out.
//!
//! Whether a checkout is there, the first that applies:
//!
//! 1. on the workspace's topic — joined here, or a branch of the topic
//!    followed from its remote — or named by the command that places it: yes;
//! 2. its presence preference, `--lazy` or `--eager`;
//! 3. `lazy = true` on every request for it: not until asked for;
//! 4. yes.
//!
//! Which form a checkout that is there gets, the first that applies:
//!
//! 1. on the workspace's topic: its sources;
//! 2. its sources cannot be read: its artefact;
//! 3. its form preference;
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

/// Whether a checkout is there while nothing asks for it: this workspace's
/// answer, over the configs' `lazy = true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    /// Left out until the topic or a command asks for it.
    Lazy,
    /// Placed, whatever the configs say.
    Eager,
}

impl fmt::Display for Presence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Presence::Lazy => "lazy",
            Presence::Eager => "eager",
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

/// The workspace's preferences, by normalised URL: the form other than
/// `source` each repository is to take, and its presence where it is not the
/// configs'. A file written before presence existed reads as having none.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default)]
    repos: BTreeMap<String, Form>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    presence: BTreeMap<String, Presence>,
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

    /// The presence preferred for `url`: `None` leaves it to the configs.
    pub fn presence(&self, url: &str) -> Option<Presence> {
        self.presence.get(&crate::urls::normalize(url)).copied()
    }

    /// Prefer `presence` for `url`; `None` leaves it to the configs again.
    pub fn set_presence(&mut self, url: &str, presence: Option<Presence>) {
        let key = crate::urls::normalize(url);
        match presence {
            Some(p) => {
                self.presence.insert(key, p);
            }
            None => {
                self.presence.remove(&key);
            }
        }
    }

    /// Every presence preference, by normalised URL.
    pub fn presences(&self) -> impl Iterator<Item = (&str, Presence)> {
        self.presence.iter().map(|(url, p)| (url.as_str(), *p))
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
    fn a_file_from_before_presence_reads() {
        let prefs: Prefs =
            toml::from_str("[repos]\n\"github.com/acme/sdk\" = \"artefact\"\n").unwrap();
        assert_eq!(
            prefs.get("https://github.com/acme/sdk"),
            Some(Form::Artefact)
        );
        assert_eq!(prefs.presence("https://github.com/acme/sdk"), None);
    }

    #[test]
    fn presence_reads_back_beside_the_form() {
        let mut prefs = Prefs::default();
        prefs.set("https://github.com/acme/sdk.git", Form::Artefact);
        prefs.set_presence("https://github.com/acme/sdk.git", Some(Presence::Lazy));
        prefs.set_presence("https://github.com/acme/core.git", Some(Presence::Eager));
        let text = toml::to_string(&prefs).unwrap();
        assert!(text.contains("[presence]"), "{}", text);
        let back: Prefs = toml::from_str(&text).unwrap();
        assert_eq!(back, prefs);
        assert_eq!(
            back.presence("git@github.com:acme/sdk.git"),
            Some(Presence::Lazy)
        );
        let mut cleared = back;
        cleared.set_presence("https://github.com/acme/core", None);
        assert_eq!(cleared.presence("https://github.com/acme/core"), None);
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

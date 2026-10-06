//! The agent skill: a short `SKILL.md` that teaches coding agents the
//! GitScale workflow, in the [Agent Skills](https://agentskills.io) format.
//!
//! Installed into `~/.agents/skills/gitscale/`, the location agents share by
//! convention, and `~/.claude/skills/gitscale/` when `~/.claude` exists —
//! Claude Code reads only its own directory. The first install is explicit:
//! a skill is instructions to agents, and nothing should add those to other
//! tools' directories unasked. Keeping it current is not: an interactive run
//! rewrites an installed skill older than the binary.
//!
//! The text is compiled in, so it always matches the version that wrote it.
//! A header after the frontmatter carries that version and a hash of the
//! rest, which settles every case without a state file: an older version is
//! rewritten, a newer one left alone, and one whose hash no longer matches
//! was edited by hand and is never touched.

use anyhow::{bail, Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const TEMPLATE: &str = include_str!("skill.md");
const VERSION: &str = env!("CARGO_PKG_VERSION");
const MARK: &str = "<!-- gitscale-skill ";
/// In the root's common git dir, under `gitscale/`: the hint was shown.
const HINT_MARKER: &str = "skill-hint";

/// The skill as this version writes it.
pub fn render() -> String {
    render_as(VERSION)
}

fn render_as(version: &str) -> String {
    let text = TEMPLATE.replace("{{version}}", version);
    let (front, body) = text
        .split_once("{{header}}\n")
        .expect("the skill template has a header line");
    let hash = crate::registry::sha256_digest(body.as_bytes());
    let hash = hash.trim_start_matches("sha256:");
    format!("{}{}{} sha256:{} -->\n{}", front, MARK, version, hash, body)
}

/// What is at one of the skill's paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Missing,
    /// Written by this version, unchanged.
    Current,
    /// Written by an older version, unchanged since.
    Older(String),
    /// Written by a newer version: an older binary leaves it be.
    Newer(String),
    /// Written by gitscale, then edited by hand.
    Modified(String),
    /// A file gitscale did not write.
    Foreign,
}

impl State {
    pub fn describe(&self) -> String {
        match self {
            State::Missing => "not installed".to_string(),
            State::Current => format!("installed, {}", VERSION),
            State::Older(v) => format!(
                "installed, {} (older: the next interactive run updates it)",
                v
            ),
            State::Newer(v) => format!("installed, {} (newer than this gitscale)", v),
            State::Modified(v) => format!("installed, {}, edited by hand: left alone", v),
            State::Foreign => "a file gitscale did not write: left alone".to_string(),
        }
    }
}

/// Where the skill goes under `home`: the shared location always, Claude
/// Code's own when it is there.
pub fn targets(home: &Path) -> Vec<PathBuf> {
    let mut paths = vec![home.join(".agents/skills/gitscale/SKILL.md")];
    if home.join(".claude").is_dir() {
        paths.push(home.join(".claude/skills/gitscale/SKILL.md"));
    }
    paths
}

/// What the file at `path` is.
pub fn inspect(path: &Path) -> State {
    let Ok(text) = fs::read_to_string(path) else {
        return State::Missing;
    };
    let Some(start) = text.find(MARK) else {
        return State::Foreign;
    };
    let rest = &text[start + MARK.len()..];
    let Some((line, body)) = rest.split_once('\n') else {
        return State::Foreign;
    };
    let mut fields = line.trim_end_matches("-->").split_whitespace();
    let (Some(version), Some(hash)) = (fields.next(), fields.next()) else {
        return State::Foreign;
    };
    let actual = crate::registry::sha256_digest(body.as_bytes());
    if hash.trim_start_matches("sha256:") != actual.trim_start_matches("sha256:") {
        return State::Modified(version.to_string());
    }
    match compare(version, VERSION) {
        std::cmp::Ordering::Less => State::Older(version.to_string()),
        std::cmp::Ordering::Equal => State::Current,
        std::cmp::Ordering::Greater => State::Newer(version.to_string()),
    }
}

fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    match (crate::version::parse(a), crate::version::parse(b)) {
        (Some(x), Some(y)) => x.compare(&y).unwrap_or(std::cmp::Ordering::Equal),
        _ => a.cmp(b),
    }
}

fn write(path: &Path) -> Result<()> {
    let dir = path.parent().expect("a skill file is in a directory");
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let partial = path.with_extension("partial");
    fs::write(&partial, render()).with_context(|| format!("cannot write {}", path.display()))?;
    fs::rename(&partial, path).with_context(|| format!("cannot write {}", path.display()))
}

/// `gitscale skill install`: write the skill to every target. A copy edited
/// by hand, or a file gitscale did not write, is replaced only with `force`.
pub fn install(home: &Path, force: bool) -> Result<Vec<(PathBuf, State)>> {
    let mut done = Vec::new();
    let planned: Vec<(PathBuf, State)> = targets(home)
        .into_iter()
        .map(|p| {
            let state = inspect(&p);
            (p, state)
        })
        .collect();
    for (path, state) in &planned {
        let why = match state {
            State::Modified(_) => "edited by hand",
            State::Foreign => "a file gitscale did not write",
            _ => continue,
        };
        if !force {
            bail!("{}: {}; pass --force to replace it", path.display(), why);
        }
    }
    for (path, state) in planned {
        if state != State::Current {
            write(&path)?;
        }
        done.push((path, state));
    }
    Ok(done)
}

/// `gitscale skill remove`: delete the skill gitscale wrote, and the
/// directory it made for it once empty. Edited copies go only with `force`;
/// a file gitscale did not write never does.
pub fn remove(home: &Path, force: bool) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for path in targets_everywhere(home) {
        match inspect(&path) {
            State::Missing => continue,
            State::Foreign => {
                bail!(
                    "{}: a file gitscale did not write; not removed",
                    path.display()
                )
            }
            State::Modified(_) if !force => bail!(
                "{}: edited by hand; pass --force to remove it",
                path.display()
            ),
            _ => {}
        }
        fs::remove_file(&path).with_context(|| format!("cannot remove {}", path.display()))?;
        if let Some(dir) = path.parent() {
            let _ = fs::remove_dir(dir);
        }
        removed.push(path);
    }
    Ok(removed)
}

/// Both locations, whether or not `~/.claude` exists now: what was installed
/// is what remove and status look at.
fn targets_everywhere(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".agents/skills/gitscale/SKILL.md"),
        home.join(".claude/skills/gitscale/SKILL.md"),
    ]
}

/// Every location and what is there.
pub fn status(home: &Path) -> Vec<(PathBuf, State)> {
    targets_everywhere(home)
        .into_iter()
        .map(|p| {
            let state = inspect(&p);
            (p, state)
        })
        .collect()
}

/// The implicit update: rewrite each installed copy older than this
/// version, unchanged since it was written. Never creates one, never
/// downgrades, never touches an edited copy. Returns what it rewrote.
pub fn refresh(home: &Path) -> Vec<PathBuf> {
    targets_everywhere(home)
        .into_iter()
        .filter(|p| matches!(inspect(p), State::Older(_)))
        .filter(|p| write(p).is_ok())
        .collect()
}

/// The one-line pointer at `skill install`, for `pull` and `status`: shown
/// when neither location holds anything, once per root — a marker in the
/// git dir every worktree of the root shares records that it was. Returns
/// whether it printed.
pub fn hint(home: &Path, root: &Path, err: &mut dyn Write) -> bool {
    if status(home)
        .iter()
        .any(|(_, state)| *state != State::Missing)
    {
        return false;
    }
    let Some(common) = crate::git::common_dir(root) else {
        return false;
    };
    let marker = common.join("gitscale").join(HINT_MARKER);
    if marker.exists() {
        return false;
    }
    let shown = fs::create_dir_all(common.join("gitscale"))
        .and_then(|()| fs::write(&marker, ""))
        .is_ok();
    // No marker means it would show on every run: better not at all.
    if shown {
        let _ = writeln!(
            err,
            "{}",
            crate::output::hint(
                "git scale skill install teaches coding agents this workflow",
                crate::output::stderr()
            )
        );
    }
    shown
}

/// The user's home, where the skill lives.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gitscale-skill-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_skill_is_an_agent_skill_with_a_header() {
        let text = render();
        assert!(
            text.starts_with("---\nname: gitscale\ndescription: "),
            "{}",
            text
        );
        assert!(text.contains(&format!("{}{} sha256:", MARK, VERSION)));
        assert!(text.contains(&format!("/blob/v{}/docs/", VERSION)));
        assert!(!text.contains("{{"));
    }

    #[test]
    fn install_goes_to_the_shared_location_and_claudes_when_present() {
        let h = home("targets");
        assert_eq!(targets(&h).len(), 1);
        fs::create_dir_all(h.join(".claude")).unwrap();
        let done = install(&h, false).unwrap();
        assert_eq!(done.len(), 2);
        for path in targets(&h) {
            assert_eq!(inspect(&path), State::Current, "{}", path.display());
        }
        let _ = fs::remove_dir_all(&h);
    }

    #[test]
    fn refresh_updates_an_older_copy_only() {
        let h = home("refresh");
        let path = h.join(".agents/skills/gitscale/SKILL.md");
        // Nothing installed: nothing created.
        assert!(refresh(&h).is_empty());
        assert!(!path.exists());

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, render_as("0.0.1")).unwrap();
        assert_eq!(inspect(&path), State::Older("0.0.1".to_string()));
        assert_eq!(refresh(&h), vec![path.clone()]);
        assert_eq!(inspect(&path), State::Current);

        // A newer one stays: an old binary never downgrades it.
        fs::write(&path, render_as("99.0.0")).unwrap();
        assert!(refresh(&h).is_empty());
        assert_eq!(inspect(&path), State::Newer("99.0.0".to_string()));

        // Edited by hand: left alone, by refresh and by install.
        let edited = render_as("0.0.1").replace("Never edit a pin", "Do not edit a pin");
        fs::write(&path, &edited).unwrap();
        assert_eq!(inspect(&path), State::Modified("0.0.1".to_string()));
        assert!(refresh(&h).is_empty());
        assert!(install(&h, false).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
        install(&h, true).unwrap();
        assert_eq!(inspect(&path), State::Current);
        let _ = fs::remove_dir_all(&h);
    }

    #[test]
    fn remove_takes_only_what_gitscale_wrote() {
        let h = home("remove");
        install(&h, false).unwrap();
        let path = h.join(".agents/skills/gitscale/SKILL.md");
        assert_eq!(remove(&h, false).unwrap(), vec![path.clone()]);
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "---\nname: gitscale\n---\nsomebody else's\n").unwrap();
        assert_eq!(inspect(&path), State::Foreign);
        assert!(remove(&h, true).is_err());
        assert!(path.exists());
        let _ = fs::remove_dir_all(&h);
    }
}

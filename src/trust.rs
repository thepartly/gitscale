//! The allowlist that decides which repositories may run `[hooks]` commands.
//!
//! `.gitscale.toml` travels inside the repository, so its `[hooks]` table is
//! written by whoever wrote the branch — including someone who has only opened
//! a merge request. A `--global` or `--system` hook install turns every `git
//! clone` and `git checkout` on the machine into a trigger for it, and that is
//! the case the allowlist exists for.
//!
//! There is no config file. The patterns are baked into the hook shim at
//! install time, next to the binary path and the chained hook that are already
//! baked in there, and the shim passes them to gitscale in
//! [`ALLOW_ENV`]. A repository cannot reach the shim, so it cannot widen its
//! own permissions; and a developer typing `gitscale pull` by hand is not
//! running under a shim, so nothing gets in their way.

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

use crate::urls;

/// Carries the allowlist from the hook shim to the gitscale it invokes.
///
/// Three states, and they mean different things:
///
/// * unset — not running under a gitscale shim. A command the user typed.
/// * set and empty — a shim that allows nothing.
/// * set — a comma-separated list of glob patterns.
pub const ALLOW_ENV: &str = "GITSCALE_HOOK_ALLOW";

/// The allowlist a `--local` install bakes in: installing a hook into one
/// repository's own `.git/hooks` is already a decision about that repository,
/// and the file cannot travel to anyone else.
pub const ALLOW_ANY: &str = "*";

// ---------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------

/// A parsed `pattern,pattern,...` allowlist.
#[derive(Debug, Clone, Default)]
pub struct Allowlist {
    patterns: Vec<String>,
}

/// Check a spec typed at `gitscale hook install --allow`.
///
/// The patterns are written into a single-quoted shell assignment in the shim,
/// so a quote would not so much escape a sandbox as quietly produce a broken
/// hook. Control characters are refused for the same reason.
pub fn validate_spec(spec: &str) -> Result<()> {
    if let Some(bad) = spec.chars().find(|c| *c == '\'' || c.is_control()) {
        bail!(
            "--allow pattern contains {:?}, which cannot be written into a hook script",
            bad
        );
    }
    if Allowlist::parse(spec).patterns.is_empty() {
        bail!("--allow was given no patterns (use '{}' to allow every repository)", ALLOW_ANY);
    }
    Ok(())
}

impl Allowlist {
    /// Split a spec into patterns. Blank entries are dropped, so a trailing
    /// comma or a spec spread over a shell line-continuation is not an error.
    pub fn parse(spec: &str) -> Self {
        Self {
            patterns: spec
                .split(',')
                .map(|p| p.trim().to_lowercase())
                .filter(|p| !p.is_empty())
                .collect(),
        }
    }

    /// The allowlist the running gitscale was invoked with, or `None` when it
    /// was not invoked by a hook shim.
    pub fn from_env() -> Option<Self> {
        std::env::var(ALLOW_ENV).ok().map(|spec| Self::parse(&spec))
    }

    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// The pattern that lets `workspace` run its hooks, if any.
    pub fn matched_by(&self, workspace: &Workspace) -> Option<&str> {
        let subjects = workspace.subjects();
        self.patterns
            .iter()
            .find(|pattern| subjects.iter().any(|s| glob_match(pattern, s)))
            .map(String::as_str)
    }

    /// The refusal, written for whoever is staring at a clone that just
    /// printed it: what was blocked, why, and what would change it.
    pub fn explain(&self, workspace: &Workspace) -> String {
        let mut msg = format!(
            "{} is not on this machine's gitscale hook allowlist.\n\n\
             A [hooks] command in .gitscale.toml runs with your shell, your SSH keys and your \
             tokens,\nand that file arrives with whatever branch is checked out — including a \
             branch you\nare only reviewing. The installed git hook runs them only for \
             repositories matching\nthe patterns it was installed with.\n",
            workspace.describe()
        );
        if self.patterns.is_empty() {
            msg.push_str("\nThe hook currently allows nothing.\n");
        } else {
            msg.push_str("\nThe hook currently allows:\n");
            for pattern in &self.patterns {
                msg.push_str(&format!("    {}\n", pattern));
            }
        }
        msg.push_str(&format!(
            "\nIf you trust this repository, reinstall the hook with it included — with \
             whichever\nscope you installed it at (--global or --system):\n\n    \
             gitscale hook install --global --allow '{}{}'\n\n\
             See `gitscale hook status` for the hooks and patterns in effect.",
            self.patterns
                .iter()
                .map(|p| format!("{},", p))
                .collect::<String>(),
            workspace.suggested_pattern()
        ));
        msg
    }
}

/// Glob match, case-insensitively, where `*` stands for any run of characters
/// and `?` for exactly one.
///
/// `*` deliberately crosses `/`: `github.com/acme/*` should cover a GitLab
/// subgroup as well as a repository directly under the owner. The flip side is
/// that a pattern must be ended deliberately — `github.com/acme*` also matches
/// `github.com/acme-evil/x`, where `github.com/acme/*` does not.
fn glob_match(pattern: &str, subject: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let sub: Vec<char> = subject.to_lowercase().chars().collect();
    let (mut p, mut s) = (0usize, 0usize);
    // Where to resume from if the current `*` turns out to have matched too
    // little: the star itself, and how much it had consumed.
    let (mut star, mut consumed) = (None, 0usize);

    while s < sub.len() {
        if p < pat.len() && (pat[p] == '?' || pat[p] == sub[s]) {
            p += 1;
            s += 1;
        } else if p < pat.len() && pat[p] == '*' {
            star = Some(p);
            consumed = s;
            p += 1;
        } else if let Some(star_at) = star {
            consumed += 1;
            s = consumed;
            p = star_at + 1;
        } else {
            return false;
        }
    }
    pat[p..].iter().all(|c| *c == '*')
}

// ---------------------------------------------------------------------------
// What is being judged
// ---------------------------------------------------------------------------

/// The directory holding a `.gitscale.toml`, and the remote it came from.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub dir: PathBuf,
    /// `origin` of the repository this directory belongs to, if any. Resolved
    /// by git, so a config in a subdirectory is judged by the repository that
    /// carries it rather than looking unowned.
    pub origin: Option<String>,
}

impl Workspace {
    pub fn probe(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            origin: crate::git::origin_url(dir),
        }
    }

    /// Every name this workspace answers to; a pattern matching any of them
    /// allows it.
    ///
    /// `host/owner/repo` is the one to write patterns against — it is the same
    /// string whether the remote was cloned over SSH or HTTPS. The raw remote
    /// and the directory are there so that a workspace with no forge URL — a
    /// local mirror, or a directory somebody assembled by hand — can still be
    /// named by a path pattern.
    pub fn subjects(&self) -> Vec<String> {
        let mut subjects = Vec::new();
        if let Some(origin) = &self.origin {
            if let Some(slug) = self.slug() {
                subjects.push(slug);
            }
            subjects.push(origin.clone());
        }
        subjects.push(self.dir.display().to_string());
        if let Ok(canonical) = self.dir.canonicalize() {
            let canonical = canonical.display().to_string();
            if !subjects.contains(&canonical) {
                subjects.push(canonical);
            }
        }
        subjects
    }

    /// `host/owner/repo`, when the remote is a URL we can decompose. Keeps the
    /// full path, so a nested GitLab subgroup stays addressable.
    pub fn slug(&self) -> Option<String> {
        let origin = self.origin.as_deref()?;
        let host = urls::extract_hostname(origin).ok()?;
        let path = urls::extract_path(origin).ok()?;
        let path = path.strip_suffix(".git").unwrap_or(&path);
        Some(format!("{}/{}", host, path).to_lowercase())
    }

    /// How to name this workspace in a one-line message.
    pub fn describe(&self) -> String {
        self.slug()
            .unwrap_or_else(|| self.dir.display().to_string())
    }

    /// The pattern to offer in a refusal: the repository itself, or the
    /// directory when there is no remote to name it by.
    fn suggested_pattern(&self) -> String {
        self.slug().unwrap_or_else(|| {
            self.dir
                .canonicalize()
                .unwrap_or_else(|_| self.dir.clone())
                .display()
                .to_string()
        })
    }
}

/// Render text that came out of a repository for a terminal: control
/// characters — escape sequences that could repaint or hide the refusal above
/// it — become visible, and a long command is cut short.
pub fn sanitize(text: &str) -> String {
    const MAX: usize = 200;
    let mut out = String::new();
    for (count, ch) in text.chars().enumerate() {
        if count == MAX {
            out.push('…');
            break;
        }
        if ch.is_control() {
            out.push_str(&format!("\\x{:02x}", ch as u32));
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(origin: &str) -> Workspace {
        Workspace {
            dir: PathBuf::from("/w"),
            origin: Some(origin.to_string()),
        }
    }

    fn allows(spec: &str, ws: &Workspace) -> bool {
        Allowlist::parse(spec).matched_by(ws).is_some()
    }

    #[test]
    fn an_empty_allowlist_denies_everything() {
        let ws = workspace("https://github.com/thepartly/gitscale.git");
        assert!(!allows("", &ws));
        assert!(allows(ALLOW_ANY, &ws));
    }

    #[test]
    fn patterns_match_at_every_granularity() {
        let ws = workspace("https://github.com/thepartly/gitscale.git");
        assert!(allows("github.com/thepartly/gitscale", &ws));
        assert!(allows("github.com/thepartly/*", &ws));
        assert!(allows("github.com/*", &ws));
        assert!(allows("*/thepartly/*", &ws));
        assert!(!allows("gitlab.com/*", &ws));
        assert!(!allows("github.com/other/*", &ws));
    }

    #[test]
    fn a_comma_separated_list_is_a_union() {
        let spec = "git.internal.example/*, github.com/thepartly/*, gitlab.com/acme/tooling";
        assert!(allows(spec, &workspace("git@git.internal.example:any/thing.git")));
        assert!(allows(spec, &workspace("https://github.com/thepartly/anything")));
        assert!(allows(spec, &workspace("https://gitlab.com/acme/tooling.git")));
        assert!(!allows(spec, &workspace("https://gitlab.com/acme/other")));
    }

    #[test]
    fn ssh_and_https_spellings_are_the_same_repository() {
        let spec = "github.com/thepartly/gitscale";
        assert!(allows(spec, &workspace("git@github.com:thepartly/gitscale.git")));
        assert!(allows(spec, &workspace("https://github.com/thepartly/gitscale.git")));
        assert!(allows(spec, &workspace("ssh://git@github.com/thepartly/gitscale")));
        // Forges treat owner and repository names case-insensitively.
        assert!(allows(spec, &workspace("https://GitHub.com/ThePartly/GitScale")));
    }

    #[test]
    fn an_owner_pattern_ends_at_the_slash() {
        // The whole point of the allowlist: "thepartly-evil" is not "thepartly".
        let ws = workspace("https://github.com/thepartly-evil/gitscale.git");
        assert!(!allows("github.com/thepartly/*", &ws));
        // And a pattern that does not end at the slash covers both, as written.
        assert!(allows("github.com/thepartly*", &ws));
    }

    #[test]
    fn a_star_crosses_slashes_so_subgroups_are_covered() {
        let ws = workspace("https://gitlab.com/acme/platform/team/service.git");
        assert!(allows("gitlab.com/acme/*", &ws));
        assert!(allows("gitlab.com/acme/platform/team/service", &ws));
    }

    #[test]
    fn a_workspace_without_a_forge_remote_is_named_by_its_path() {
        let local = Workspace {
            dir: PathBuf::from("/srv/workspaces/core"),
            origin: None,
        };
        assert!(!allows("github.com/*", &local));
        assert!(allows("/srv/workspaces/*", &local));
        assert!(allows("/srv/workspaces/core", &local));
    }

    #[test]
    fn a_local_mirror_can_be_named_by_its_remote_path() {
        let ws = Workspace {
            dir: PathBuf::from("/w"),
            origin: Some("/srv/mirrors/core.git".to_string()),
        };
        assert!(allows("/srv/mirrors/*", &ws));
        assert!(!allows("github.com/*", &ws));
    }

    #[test]
    fn specs_that_would_break_the_shim_are_refused() {
        assert!(validate_spec("github.com/thepartly/*").is_ok());
        assert!(validate_spec("a'; rm -rf /; echo '").is_err());
        assert!(validate_spec("github.com/a/*\nrm -rf /").is_err());
        assert!(validate_spec("  ,  ").is_err());
    }

    #[test]
    fn a_refusal_names_the_repo_and_the_command_that_would_allow_it() {
        let ws = workspace("https://github.com/thepartly/gitscale.git");
        let msg = Allowlist::parse("github.com/other/*").explain(&ws);
        assert!(msg.contains("github.com/thepartly/gitscale"));
        assert!(msg.contains("--allow 'github.com/other/*,github.com/thepartly/gitscale'"));
    }

    #[test]
    fn sanitize_defangs_escape_sequences() {
        let sanitized = sanitize("curl evil\x1b[2K\rsomething harmless");
        assert!(!sanitized.contains('\x1b'));
        assert!(sanitized.contains("\\x1b"));
        assert!(sanitize(&"a".repeat(500)).chars().count() <= 201);
    }
}

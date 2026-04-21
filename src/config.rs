use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

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
}

impl RepoEntry {
    pub fn is_readonly(&self) -> bool {
        matches!(self.mode, RepoMode::Readonly | RepoMode::Artefact)
    }

    pub fn is_artefact(&self) -> bool {
        matches!(self.mode, RepoMode::Artefact)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Hooks {
    pub post_sync: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GitScaleConfig {
    pub repos: Vec<RepoEntry>,
    pub storage_url: String,
    pub hooks: Hooks,
}

#[derive(Deserialize)]
struct RawConfig {
    storage: Option<RawStorage>,
    repos: Option<BTreeMap<String, RawRepo>>,
    hooks: Option<RawHooks>,
}

#[derive(Deserialize)]
struct RawHooks {
    post_sync: Option<String>,
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
    let hooks = parse_hooks(raw.hooks.as_ref());

    Ok(GitScaleConfig {
        repos,
        storage_url,
        hooks,
    })
}

fn parse_storage(raw: Option<&RawStorage>, config_path: &Path) -> Result<String> {
    let Some(storage) = raw else {
        return Ok(String::new());
    };
    match &storage.url {
        Some(u) if !u.is_empty() => Ok(u.trim_end_matches('/').to_string()),
        _ => bail!("{}: storage.url is required", config_path.display()),
    }
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

        let revision = spec.revision.clone().unwrap_or_default();

        let mode = match &spec.mode {
            Some(m) => RepoMode::from_str_checked(m)
                .with_context(|| format!("{}: repos.{}.mode", config_path.display(), directory))?,
            None => RepoMode::Readwrite,
        };

        entries.push(RepoEntry {
            directory: directory.clone(),
            repo_url: url.to_string(),
            revision,
            mode,
        });
    }
    Ok(entries)
}

fn parse_hooks(raw: Option<&RawHooks>) -> Hooks {
    let Some(hooks) = raw else {
        return Hooks::default();
    };
    Hooks {
        post_sync: hooks.post_sync.clone(),
    }
}

pub fn write_config(config_path: &Path, entries: &[RepoEntry], storage_url: &str) -> Result<()> {
    let mut lines: Vec<String> = Vec::new();

    if !storage_url.is_empty() {
        lines.push("[storage]".to_string());
        lines.push(format!("url = \"{}\"", storage_url));
        lines.push(String::new());
    }

    if !entries.is_empty() {
        lines.push("[repos]".to_string());
        for entry in entries {
            let mut parts = vec![format!("url = \"{}\"", entry.repo_url)];
            if !entry.revision.is_empty() {
                parts.push(format!("revision = \"{}\"", entry.revision));
            }
            if entry.mode != RepoMode::Readwrite {
                parts.push(format!("mode = \"{}\"", entry.mode));
            }
            let inline = parts.join(", ");
            lines.push(format!("\"{}\" = {{ {} }}", entry.directory, inline));
        }
    }

    lines.push(String::new()); // trailing newline
    std::fs::write(config_path, lines.join("\n"))
        .with_context(|| format!("cannot write {}", config_path.display()))?;
    Ok(())
}

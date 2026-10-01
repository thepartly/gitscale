//! Where resolution reads repositories from, without ever fetching history it
//! would not otherwise have.
//!
//! What resolution needs is small: each repository's branches and tags, and
//! the `.gitscale.toml` of each selected commit. So:
//!
//! * **Mirror** — on a developer machine with the cache on, a source
//!   repository is read from its cache mirror: the entry every checkout is
//!   about to be built from anyway, updated once per command. Full history is
//!   there, which is the one place the `ahead` warning can be computed.
//! * **Light** — everywhere else (`--no-cache`, CI, a repository only ever
//!   used as an artefact) a bare repository in the workspace's own git
//!   directory, holding no history: refs come from `ls-remote` and are kept
//!   beside it, and a commit's config arrives with a depth-1, blobless fetch
//!   of that one commit — or, in CI with the cache on, from the snapshot pin
//!   the checkout will be built from anyway.
//!
//! Offline (`status` without `--fetch`), nothing is fetched: what is on disk
//! answers, and what it cannot is [`Unavailable`].

use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::artefact::Artefacts;
use crate::cache::Cache;
use crate::config::CONFIG_FILENAME;
use crate::resolution::{unavailable, Checkouts, Kind, Refs, Repos, Unavailable};

/// The refs a light store last saw, beside it.
const REFS_FILE: &str = "gitscale-refs.json";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Store {
    Mirror,
    Light,
}

pub struct GitStores<'a> {
    online: bool,
    ci: bool,
    cache: Option<Cache>,
    /// Where the workspace's light stores live.
    local: PathBuf,
    artefacts: Option<&'a Artefacts>,
    verbose: bool,
    /// The refs of each store, read or fetched once per command, by path.
    refs: Mutex<HashMap<PathBuf, std::result::Result<Refs, String>>>,
    /// Mirrors whose default branch this command has already asked the
    /// remote for.
    asked_default: Mutex<std::collections::HashSet<PathBuf>>,
}

impl<'a> GitStores<'a> {
    pub fn new(
        config_root: &Path,
        cache: Option<Cache>,
        online: bool,
        artefacts: Option<&'a Artefacts>,
        verbose: bool,
    ) -> Self {
        GitStores {
            online,
            ci: crate::git::is_ci(),
            cache,
            local: local_dir(config_root),
            artefacts,
            verbose,
            refs: Mutex::new(HashMap::new()),
            asked_default: Mutex::new(Default::default()),
        }
    }

    /// Which store `url` is read from. Offline, a workspace whose checkouts
    /// were made without the cache has no mirror to read, but its own store.
    fn store(&self, url: &str, kind: Kind) -> Store {
        let preferred = match (&self.cache, kind) {
            (Some(_), Kind::Source) if !self.ci => Store::Mirror,
            _ => Store::Light,
        };
        if preferred == Store::Mirror && !self.online {
            let mirror = self.mirror_path(url);
            let light = self.light_path(url);
            if !is_store(&mirror) && light.join(REFS_FILE).is_file() {
                return Store::Light;
            }
        }
        preferred
    }

    fn mirror_path(&self, url: &str) -> PathBuf {
        match &self.cache {
            Some(cache) => cache.mirror_path(&crate::ci::remote_url(url)),
            None => self.light_path(url),
        }
    }

    fn light_path(&self, url: &str) -> PathBuf {
        self.local.join(crate::cache::entry_name(url))
    }

    fn path(&self, url: &str, kind: Kind) -> PathBuf {
        match self.store(url, kind) {
            Store::Mirror => self.mirror_path(url),
            Store::Light => self.light_path(url),
        }
    }

    /// Fetch, or read, the refs of `url`'s store.
    fn load_refs(&self, url: &str, kind: Kind, path: &Path) -> Result<Refs> {
        let remote = crate::ci::remote_url(url);
        let store = self.store(url, kind);
        if self.online {
            if self.verbose {
                eprintln!("  resolve  {}", url);
            }
            let fetched = match (store, &self.cache) {
                (Store::Mirror, Some(cache)) => cache.mirror(&remote).map(|_| ()),
                _ => refresh_light(&remote, path),
            };
            if let Err(e) = fetched {
                let had = match store {
                    Store::Mirror => is_store(path),
                    Store::Light => path.join(REFS_FILE).is_file(),
                };
                if !had {
                    return Err(e).with_context(|| format!("cannot fetch {}", url));
                }
                // What the last fetch saw still resolves, if not to the latest.
                eprintln!(
                    "  resolve  {}: {:#} (using what was fetched before)",
                    url, e
                );
            }
        }
        match store {
            Store::Mirror if is_store(path) => mirror_refs(path),
            Store::Light if path.join(REFS_FILE).is_file() => {
                let text = fs::read_to_string(path.join(REFS_FILE))?;
                serde_json::from_str(&text)
                    .with_context(|| format!("{}: unreadable", path.join(REFS_FILE).display()))
            }
            _ => Err(unavailable(format!(
                "{} has not been fetched on this machine yet",
                url
            ))),
        }
    }

    /// Run git in a store. Offline, any fetch git would make for a missing
    /// object is refused rather than made.
    fn git(&self, store: &Path, args: &[&str]) -> Result<std::process::Output> {
        let dir = store.to_string_lossy().to_string();
        let mut full: Vec<&str> = vec!["-C", &dir];
        if !self.online {
            full.extend_from_slice(&["-c", "protocol.allow=never"]);
        }
        full.extend_from_slice(args);
        crate::git::run_git(&full, None, false)
    }

    /// `.gitscale.toml` at `commit` in the repository at `path`: `Some(None)`
    /// when the commit is there without one, `None` when the commit (or the
    /// file's object) is not there to read.
    fn show(&self, path: &Path, commit: &str) -> Result<Option<Option<String>>> {
        let shown = self.git(path, &["show", &format!("{}:{}", commit, CONFIG_FILENAME)])?;
        if shown.status.success() {
            return Ok(Some(Some(
                String::from_utf8_lossy(&shown.stdout).into_owned(),
            )));
        }
        let tree = format!("{}^{{tree}}", commit);
        if self.git(path, &["cat-file", "-e", &tree])?.status.success() {
            // The commit's tree is there, so the file is not — unless it is a
            // blob a blobless fetch left behind, which `show` just failed to
            // fetch.
            let listed = self.git(path, &["ls-tree", "--name-only", commit, CONFIG_FILENAME])?;
            if String::from_utf8_lossy(&listed.stdout).trim().is_empty() {
                return Ok(Some(None));
            }
        }
        Ok(None)
    }

    fn not_here(&self, url: &str, commit: &str) -> anyhow::Error {
        let what = format!(
            "{} at {} of {}",
            CONFIG_FILENAME,
            crate::git::short_sha(commit),
            url
        );
        if self.online {
            anyhow::anyhow!("cannot fetch {}", what)
        } else {
            unavailable(format!("{} is not on this machine", what))
        }
    }
}

impl Repos for GitStores<'_> {
    fn refs(&self, url: &str, kind: Kind) -> Result<Refs> {
        let path = self.path(url, kind);
        if let Some(known) = self.refs.lock().unwrap().get(&path) {
            return known.clone().map_err(|e| anyhow::anyhow!(e));
        }
        let loaded = self.load_refs(url, kind, &path);
        match &loaded {
            Err(e) if e.downcast_ref::<Unavailable>().is_some() => {}
            Ok(refs) => {
                self.refs.lock().unwrap().insert(path, Ok(refs.clone()));
            }
            Err(e) => {
                self.refs
                    .lock()
                    .unwrap()
                    .insert(path, Err(format!("{:#}", e)));
            }
        }
        loaded
    }

    fn config_at(&self, url: &str, commit: &str, revision: &str) -> Result<Option<String>> {
        let remote = crate::ci::remote_url(url);
        match (self.store(url, Kind::Source), &self.cache) {
            (Store::Mirror, Some(cache)) => {
                let path = cache.mirror_path(&remote);
                if let Some(found) = self.show(&path, commit)? {
                    return Ok(found);
                }
                if self.online {
                    // A commit no branch or tag reaches: ask for it by name.
                    let _ = self.git(&path, &["fetch", "--quiet", "origin", commit]);
                    if let Some(found) = self.show(&path, commit)? {
                        return Ok(found);
                    }
                }
                Err(self.not_here(url, commit))
            }
            (_, cache) => {
                // CI with the cache on: the snapshot pin the checkout is built
                // from holds exactly this commit.
                if let (Some(cache), true) = (cache, self.ci) {
                    let snapshot = cache.snapshot_path(&remote);
                    if is_store(&snapshot) {
                        if let Some(found) = self.show(&snapshot, commit)? {
                            return Ok(found);
                        }
                    }
                    if self.online {
                        if let Ok(Some(pinned)) = cache.pin(&remote, commit) {
                            if let Some(found) = self.show(&pinned.entry, commit)? {
                                return Ok(found);
                            }
                        }
                    }
                }
                let path = self.light_path(url);
                if is_store(&path) {
                    if let Some(found) = self.show(&path, commit)? {
                        return Ok(found);
                    }
                }
                if !self.online {
                    return Err(self.not_here(url, commit));
                }
                ensure_light(&remote, &path)?;
                fetch_one(&path, commit, revision);
                match self.show(&path, commit)? {
                    Some(found) => Ok(found),
                    None => Err(self.not_here(url, commit)),
                }
            }
        }
    }

    fn artefact_config(&self, url: &str, commit: &str) -> Result<Option<String>> {
        let Some(artefacts) = self.artefacts else {
            return Err(unavailable(format!(
                "the artefact config of {} was not looked up",
                url
            )));
        };
        // A commit whose pipeline has not published yet, a registry that is
        // down: the entry's own install reports that, in its own words. For
        // resolution it is a checkout whose dependencies cannot be read.
        artefacts
            .config_layer(url, commit, self.online)
            .map_err(|e| match e.downcast_ref::<Unavailable>() {
                Some(_) => e,
                None => unavailable(format!("{:#}", e)),
            })
    }

    fn default_branch(&self, url: &str, kind: Kind) -> Result<Option<String>> {
        let path = self.path(url, kind);
        if self.store(url, kind) == Store::Mirror
            && self.online
            && self.asked_default.lock().unwrap().insert(path.clone())
        {
            // A mirror's fetch does not carry the remote's HEAD: asked for
            // once, the first time a command needs it, and kept as the
            // mirror's own HEAD for every read after.
            set_default_branch(&crate::ci::remote_url(url), &path);
            self.refs.lock().unwrap().remove(&path);
        }
        Ok(self.refs(url, kind)?.default_branch)
    }

    fn is_ancestor(&self, url: &str, kind: Kind, ancestor: &str, descendant: &str) -> Result<bool> {
        // Only a mirror has history; nothing else fetches it for a warning.
        if self.store(url, kind) != Store::Mirror {
            return Err(unavailable("no history on this machine"));
        }
        let path = self.path(url, kind);
        let checked = self.git(
            &path,
            &["merge-base", "--is-ancestor", ancestor, descendant],
        )?;
        match checked.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(unavailable("the history is not on this machine")),
        }
    }

    fn prepare(&self, wanted: &[(String, Kind)]) {
        // Refs of every repository, fetched side by side: the network wait
        // is what resolution costs, and it is per repository.
        let mut todo: BTreeMap<PathBuf, (String, Kind)> = BTreeMap::new();
        {
            let known = self.refs.lock().unwrap();
            for (url, kind) in wanted {
                let path = self.path(url, *kind);
                if !known.contains_key(&path) {
                    todo.insert(path, (url.clone(), *kind));
                }
            }
        }
        if todo.len() < 2 {
            return;
        }
        std::thread::scope(|scope| {
            for (url, kind) in todo.values() {
                scope.spawn(move || {
                    let _ = self.refs(url, *kind);
                });
            }
        });
    }
}

/// Where the workspace keeps its own stores: in its git directory when it is
/// the top of a repository, so they never show up as untracked files, else
/// beside its config.
fn local_dir(config_root: &Path) -> PathBuf {
    let in_git = if crate::git::is_repo_root(config_root) {
        crate::git::git_path(config_root, "gitscale/resolve")
    } else {
        None
    };
    in_git.unwrap_or_else(|| config_root.join(".gitscale/resolve"))
}

fn is_store(path: &Path) -> bool {
    path.join("HEAD").is_file()
}

/// A mirror's branches and tags, each peeled to its commit, and the default
/// branch its HEAD names.
fn mirror_refs(path: &Path) -> Result<Refs> {
    let dir = path.to_string_lossy().to_string();
    let listed = crate::git::run_git(
        &[
            "-C",
            &dir,
            "for-each-ref",
            "--format=%(refname)%09%(objectname)%09%(*objectname)",
            "refs/heads",
            "refs/tags",
        ],
        None,
        true,
    )?;
    let mut refs = Refs::default();
    for line in String::from_utf8_lossy(&listed.stdout).lines() {
        let mut fields = line.split('\t');
        let (Some(name), Some(object), peeled) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let commit = peeled
            .filter(|p| !p.is_empty())
            .unwrap_or(object)
            .to_string();
        if let Some(branch) = name.strip_prefix("refs/heads/") {
            refs.branches.insert(branch.to_string(), commit);
        } else if let Some(tag) = name.strip_prefix("refs/tags/") {
            refs.tags.insert(tag.to_string(), commit);
        }
    }
    refs.default_branch = crate::git::query(path, &["symbolic-ref", "--short", "HEAD"])
        .filter(|b| refs.branches.contains_key(b));
    Ok(refs)
}

/// Create the light store for `url` at `path` if it is not there: an empty
/// bare repository, `origin` pointing at the remote.
fn ensure_light(url: &str, path: &Path) -> Result<()> {
    if is_store(path) {
        crate::git::set_origin(path, url)?;
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("a store has no parent directory"))?;
    fs::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;
    // Built under another name and moved into place, so an interrupted
    // creation is never mistaken for a store.
    let staging = path.with_extension("staging");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let dir = staging.to_string_lossy().to_string();
    crate::git::run_git(&["-C", &dir, "init", "--bare", "--quiet"], None, true)?;
    crate::git::run_git(&["-C", &dir, "remote", "add", "origin", url], None, true)?;
    fs::rename(&staging, path)
        .with_context(|| format!("cannot move the new store into {}", path.display()))?;
    Ok(())
}

/// The remote's branches, tags and default branch, from one ref
/// advertisement each — no objects — kept beside the light store for the
/// next offline read.
fn refresh_light(url: &str, path: &Path) -> Result<()> {
    ensure_light(url, path)?;
    let mut refs = Refs::default();
    let (listed, default) = crate::git::ls_remote_full(url)?;
    for (commit, name) in listed {
        if let Some(branch) = name.strip_prefix("refs/heads/") {
            refs.branches.insert(branch.to_string(), commit);
        } else if let Some(tag) = name.strip_prefix("refs/tags/") {
            refs.tags.insert(tag.to_string(), commit);
        }
    }
    refs.default_branch = default.filter(|b| refs.branches.contains_key(b));
    let text = serde_json::to_string_pretty(&refs)?;
    let file = path.join(REFS_FILE);
    let partial = file.with_extension("partial");
    fs::write(&partial, text)?;
    fs::rename(&partial, &file)?;
    Ok(())
}

/// Bring one commit into a light store: depth 1, no files until one is read.
/// By its SHA, and by the ref that names it for a remote that will not serve
/// a bare commit. Best effort: what is still missing afterwards is reported
/// by the read.
fn fetch_one(path: &Path, commit: &str, revision: &str) {
    let dir = path.to_string_lossy().to_string();
    let fetch = |what: &str| {
        crate::git::run_git(
            &[
                "-C",
                &dir,
                "fetch",
                "--quiet",
                "--no-tags",
                "--depth",
                "1",
                "--filter=blob:none",
                "origin",
                what,
            ],
            None,
            false,
        )
        .is_ok_and(|o| o.status.success())
    };
    if fetch(commit) || revision.is_empty() || crate::git::is_full_sha(revision) {
        return;
    }
    let _ = fetch(revision);
}

/// The remote's default branch, from its HEAD symref.
fn default_branch(url: &str) -> Option<String> {
    let listed = crate::git::run_git(&["ls-remote", "--symref", url, "HEAD"], None, false).ok()?;
    let text = String::from_utf8_lossy(&listed.stdout);
    text.lines().find_map(|line| {
        line.strip_prefix("ref: refs/heads/")?
            .split_once('\t')
            .filter(|(_, name)| *name == "HEAD")
            .map(|(branch, _)| branch.to_string())
    })
}

/// Point the mirror's HEAD at the remote's default branch, which is what a
/// bare repository's HEAD means. Best effort.
fn set_default_branch(url: &str, path: &Path) {
    if let Some(branch) = default_branch(url) {
        let dir = path.to_string_lossy().to_string();
        let target = format!("refs/heads/{}", branch);
        let _ = crate::git::run_git(&["-C", &dir, "symbolic-ref", "HEAD", &target], None, false);
    }
}

/// The workspace's checkouts as resolution sees them.
pub struct WorkspaceCheckouts {
    root: PathBuf,
}

impl WorkspaceCheckouts {
    pub fn new(root: &Path) -> Self {
        WorkspaceCheckouts {
            root: root.to_path_buf(),
        }
    }
}

impl Checkouts for WorkspaceCheckouts {
    fn config_if_at(&self, directory: &str, kind: Kind, commit: &str) -> Option<Option<String>> {
        let dest = self.root.join(directory);
        let at = match kind {
            Kind::Source => {
                crate::git::is_checkout(&dest)
                    && crate::git::resolve_ref(&dest, "HEAD").as_deref() == Some(commit)
            }
            Kind::Artefact => {
                crate::artefact::installed_commit(&self.root, directory).as_deref() == Some(commit)
            }
        };
        if !at {
            return None;
        }
        Some(fs::read_to_string(dest.join(CONFIG_FILENAME)).ok())
    }

    fn head(&self, directory: &str) -> Option<String> {
        let dest = self.root.join(directory);
        if !crate::git::is_checkout(&dest) {
            return None;
        }
        crate::git::resolve_ref(&dest, "HEAD")
    }
}

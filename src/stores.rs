//! Where resolution reads repositories from, without ever fetching history it
//! would not otherwise have.
//!
//! What resolution needs is small: each repository's branches and tags, and
//! the `.gitscale.toml` of each selected commit. So:
//!
//! * **Repo** — on a developer machine, a source repository is read from the
//!   root's own store for it: the repository every checkout of it is a
//!   worktree of, fetched once per command. Full history is there, which is
//!   the one place the `ahead` warning can be computed — and its own branches
//!   are where topics are developed.
//! * **Light** — everywhere else (CI, a repository only ever used as an
//!   artefact) a bare repository in the workspace's own git directory,
//!   holding no history: refs come from `ls-remote` and are kept beside it,
//!   and a commit's config arrives with a depth-1, blobless fetch of that one
//!   commit — or, in CI with the cache on, from the snapshot pin the checkout
//!   will be built from anyway.
//!
//! Offline (`ls` without `--fetch`), nothing is fetched: what is on disk
//! answers, and what it cannot is [`Unavailable`]. Fetching on a miss starts
//! offline and fetches only the stores that lacked something: see
//! [`OnMiss`].

use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::artefact::Artefacts;
use crate::config::CONFIG_FILENAME;
use crate::resolution::{unavailable, Checkouts, Refs, Repos, Unavailable};
use crate::store::Sources;

/// The refs a light store last saw, beside it.
const REFS_FILE: &str = "gitscale-refs.json";
/// The releases a registry has of a repository whose sources cannot be read,
/// kept for the next offline read.
const RELEASED_FILE: &str = "gitscale-released.json";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Store {
    Repo,
    Light,
}

/// What fetching on a miss shares between its rounds: the stores to fetch
/// although the rest are read offline, the ones already fetched, and the
/// ones the last round found lacking.
#[derive(Default)]
pub struct OnMiss {
    fetch: Mutex<std::collections::BTreeSet<PathBuf>>,
    fetched: Mutex<std::collections::BTreeSet<PathBuf>>,
    missed: Mutex<std::collections::BTreeSet<PathBuf>>,
}

impl OnMiss {
    /// The stores the last round lacked something in that are not fetched
    /// yet, now marked to be: empty when another round would change nothing.
    pub fn next_round(&self) -> usize {
        let missed = std::mem::take(&mut *self.missed.lock().unwrap());
        let mut fetch = self.fetch.lock().unwrap();
        let before = fetch.len();
        fetch.extend(missed);
        fetch.len() - before
    }
}

pub struct GitStores<'a> {
    online: bool,
    /// Offline, but fetching each store that lacks something.
    on_miss: Option<&'a OnMiss>,
    sources: &'a Sources,
    /// Where the workspace's light stores live.
    local: PathBuf,
    artefacts: Option<&'a Artefacts>,
    verbose: bool,
    /// The refs of each store, read or fetched once per command, by path.
    refs: Mutex<HashMap<PathBuf, std::result::Result<Refs, String>>>,
    /// Repositories whose sources cannot be read, by normalized URL: read
    /// from their registry instead.
    sourceless: Mutex<BTreeSet<String>>,
    /// Repositories this workspace takes as artefacts, by normalized URL.
    artefact: BTreeSet<String>,
}

impl<'a> GitStores<'a> {
    pub fn new(
        config_root: &Path,
        sources: &'a Sources,
        online: bool,
        artefacts: Option<&'a Artefacts>,
        verbose: bool,
    ) -> Self {
        GitStores {
            online,
            on_miss: None,
            sources,
            local: local_dir(config_root),
            artefacts,
            verbose,
            refs: Mutex::new(HashMap::new()),
            sourceless: Mutex::new(BTreeSet::new()),
            artefact: crate::prefer::Prefs::load(config_root)
                .map(|prefs| {
                    prefs
                        .iter()
                        .filter(|(_, form)| *form == crate::prefer::Form::Artefact)
                        .map(|(url, _)| url.to_string())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// Read offline, fetching only what `on_miss` marks.
    pub fn fetching_on_miss(mut self, on_miss: &'a OnMiss) -> Self {
        self.online = false;
        self.on_miss = Some(on_miss);
        self
    }

    /// Whether the store at `path` is asked online.
    fn online_for(&self, path: &Path) -> bool {
        self.online
            || self
                .on_miss
                .is_some_and(|m| m.fetch.lock().unwrap().contains(path))
    }

    /// Note that the store at `path` lacked something: the next round of
    /// fetching on a miss fetches it.
    fn missed(&self, path: &Path) {
        if let Some(on_miss) = self.on_miss {
            on_miss.missed.lock().unwrap().insert(path.to_path_buf());
        }
    }

    /// Which store a repository is read from: the root's own store for it,
    /// or a light one — in CI, which keeps none, and for a repository taken
    /// as an artefact, which needs its refs listed and nothing downloaded,
    /// unless it has a store from being developed here.
    fn store(&self, url: &str) -> Store {
        match &self.sources.stores {
            Some(stores)
                if !self.artefact.contains(&crate::urls::normalize(url))
                    || is_store(&stores.repo_path(&crate::ci::remote_url(url))) =>
            {
                Store::Repo
            }
            _ => Store::Light,
        }
    }

    /// Whether this workspace takes `url` as an artefact: its configs are
    /// then read from the images of its releases.
    fn is_artefact(&self, url: &str) -> bool {
        self.sourceless(url) || self.artefact.contains(&crate::urls::normalize(url))
    }

    fn repo_path(&self, url: &str) -> PathBuf {
        match &self.sources.stores {
            Some(stores) => stores.repo_path(url),
            None => self.light_path(url),
        }
    }

    fn light_path(&self, url: &str) -> PathBuf {
        self.local.join(crate::store::entry_name(url))
    }

    fn path(&self, url: &str) -> PathBuf {
        match self.store(url) {
            Store::Repo => self.repo_path(url),
            Store::Light => self.light_path(url),
        }
    }

    /// Fetch, or read, the refs of `url`'s store.
    fn load_refs(&self, url: &str, path: &Path) -> Result<Refs> {
        let remote = crate::ci::remote_url(url);
        let store = self.store(url);
        // Fetched once however many rounds ask: a light store has no record
        // of its own of being fetched by this command.
        let done = self
            .on_miss
            .is_some_and(|m| m.fetched.lock().unwrap().contains(path));
        let released = self.light_path(url).join(RELEASED_FILE);
        if self.online_for(path) && !done {
            if self.verbose {
                eprintln!("  resolve  {}", url);
            }
            // Before the fetch, which makes a store it may then fail to fill.
            let had = match store {
                Store::Repo => is_store(path),
                Store::Light => path.join(REFS_FILE).is_file(),
            };
            let fetched = match (store, &self.sources.stores) {
                (Store::Repo, Some(stores)) => stores.update(&remote).map(|_| ()),
                _ => refresh_light(&remote, path).with_context(|| format!("cannot fetch {}", url)),
            };
            if let Some(on_miss) = self.on_miss {
                on_miss.fetched.lock().unwrap().insert(path.to_path_buf());
            }
            if fetched.is_ok() {
                // Readable again: what its registry said is no longer the
                // answer.
                let _ = fs::remove_file(&released);
            }
            if let Err(e) = fetched {
                // Refused rather than unreachable: the registry may have what
                // the sources would have said.
                if crate::git::is_access_error(&format!("{:#}", e)) {
                    match self.registry_refs(url) {
                        Ok(Some(refs)) => return Ok(refs),
                        Ok(None) => {}
                        Err(registry) => {
                            bail!("{:#}, nor read its registry: {:#}", e, registry)
                        }
                    }
                }
                // In CI a runner keeps the build directory between jobs: the
                // refs an earlier job fetched would build a stale commit.
                if !had || crate::git::is_ci() {
                    return Err(e);
                }
                // What the last fetch saw still resolves, if not to the latest.
                eprintln!(
                    "  resolve  {}: {:#} (using what was fetched before)",
                    url, e
                );
            }
        }
        if released.is_file() {
            self.sourceless
                .lock()
                .unwrap()
                .insert(crate::urls::normalize(url));
            let text = fs::read_to_string(&released)?;
            return serde_json::from_str(&text)
                .with_context(|| format!("{}: unreadable", released.display()));
        }
        match store {
            Store::Repo if is_store(path) => repo_refs(path),
            Store::Light if path.join(REFS_FILE).is_file() => {
                let text = fs::read_to_string(path.join(REFS_FILE))?;
                serde_json::from_str(&text)
                    .with_context(|| format!("{}: unreadable", path.join(REFS_FILE).display()))
            }
            _ => {
                self.missed(path);
                Err(unavailable(format!(
                    "{} has not been fetched on this machine yet",
                    url
                )))
            }
        }
    }

    /// The releases `url`'s registry has, as refs: its version tags, each at
    /// the commit its image was built from, and no branches. `None` when the
    /// registry has none to offer.
    fn registry_refs(&self, url: &str) -> Result<Option<Refs>> {
        let Some(artefacts) = self.artefacts else {
            return Ok(None);
        };
        let tags = artefacts.released(url)?;
        if tags.is_empty() {
            return Ok(None);
        }
        let refs = Refs {
            tags,
            ..Refs::default()
        };
        let dir = self.light_path(url);
        fs::create_dir_all(&dir)?;
        let file = dir.join(RELEASED_FILE);
        let partial = file.with_extension("partial");
        fs::write(&partial, serde_json::to_string_pretty(&refs)?)?;
        fs::rename(&partial, &file)?;
        self.sourceless
            .lock()
            .unwrap()
            .insert(crate::urls::normalize(url));
        Ok(Some(refs))
    }

    /// Run git in a store. Offline, any fetch git would make for a missing
    /// object is refused rather than made.
    fn git(&self, store: &Path, args: &[&str]) -> Result<std::process::Output> {
        let dir = store.to_string_lossy().to_string();
        let mut full: Vec<&str> = vec!["-C", &dir];
        if !self.online_for(store) {
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

    fn not_here(&self, url: &str, commit: &str, path: &Path) -> anyhow::Error {
        let what = format!(
            "{} at {} of {}",
            CONFIG_FILENAME,
            crate::git::short_sha(commit),
            url
        );
        if self.online_for(path) {
            anyhow::anyhow!("cannot fetch {}", what)
        } else {
            self.missed(path);
            unavailable(format!("{} is not on this machine", what))
        }
    }
}

impl GitStores<'_> {
    /// The `.gitscale.toml` the image of the release `tag` carries, for a
    /// repository taken as an artefact.
    fn image_config(&self, url: &str, tag: &str) -> Result<Option<String>> {
        let Some(artefacts) = self.artefacts else {
            return Err(unavailable(format!(
                "the image config of {} was not looked up",
                url
            )));
        };
        // Keyed apart from the git stores: the registry is asked, not git.
        let key = PathBuf::from(format!("artefact:{}", crate::urls::normalize(url)));
        let online = self.online_for(&key);
        artefacts.config(url, tag, online).map_err(|e| {
            if e.downcast_ref::<Unavailable>().is_some() && !online {
                self.missed(&key);
                return e;
            }
            unavailable(format!("{:#}", e))
        })
    }
}

impl Repos for GitStores<'_> {
    fn refs(&self, url: &str) -> Result<Refs> {
        let path = self.path(url);
        if let Some(known) = self.refs.lock().unwrap().get(&path) {
            return known.clone().map_err(|e| anyhow::anyhow!(e));
        }
        let loaded = self.load_refs(url, &path);
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
        if let (true, Some(tag)) = (self.is_artefact(url), crate::artefact::image_tag(revision)) {
            return self.image_config(url, tag);
        }
        let remote = crate::ci::remote_url(url);
        match self.store(url) {
            Store::Repo => {
                let path = self.repo_path(url);
                let online = self.online_for(&path);
                if online && !is_store(&path) {
                    if let Some(stores) = &self.sources.stores {
                        stores.update(&remote)?;
                    }
                }
                if let Some(found) = self.show(&path, commit)? {
                    return Ok(found);
                }
                if online {
                    // A commit no branch or tag reaches: ask for it by name.
                    let _ = self.git(&path, &["fetch", "--quiet", "origin", commit]);
                    if let Some(found) = self.show(&path, commit)? {
                        return Ok(found);
                    }
                }
                Err(self.not_here(url, commit, &path))
            }
            Store::Light => {
                let path = self.light_path(url);
                let online = self.online_for(&path);
                // CI with the cache on: the snapshot pin the checkout is built
                // from holds exactly this commit.
                if let Some(cache) = &self.sources.cache {
                    let snapshot = cache.snapshot_path(&remote);
                    if is_store(&snapshot) {
                        if let Some(found) = self.show(&snapshot, commit)? {
                            return Ok(found);
                        }
                    }
                    if online {
                        if let Ok(Some(pinned)) = cache.pin(&remote, commit) {
                            if let Some(found) = self.show(&pinned.entry, commit)? {
                                return Ok(found);
                            }
                        }
                    }
                }
                if is_store(&path) {
                    if let Some(found) = self.show(&path, commit)? {
                        return Ok(found);
                    }
                }
                if !online {
                    return Err(self.not_here(url, commit, &path));
                }
                ensure_light(&remote, &path)?;
                fetch_one(&path, commit, revision);
                match self.show(&path, commit)? {
                    Some(found) => Ok(found),
                    None => Err(self.not_here(url, commit, &path)),
                }
            }
        }
    }

    fn build(&self, url: &str, hash: &str) -> Result<String> {
        let Some(artefacts) = self.artefacts else {
            bail!("the builds of {} are not looked up here", url);
        };
        let key = PathBuf::from(format!("artefact:{}", crate::urls::normalize(url)));
        let online = self.online_for(&key);
        artefacts.built_from(url, hash, online).inspect_err(|e| {
            if e.downcast_ref::<Unavailable>().is_some() && !online {
                self.missed(&key);
            }
        })
    }

    fn sourceless(&self, url: &str) -> bool {
        self.sourceless
            .lock()
            .unwrap()
            .contains(&crate::urls::normalize(url))
    }

    fn default_branch(&self, url: &str) -> Result<Option<String>> {
        Ok(self.refs(url)?.default_branch)
    }

    fn is_ancestor(&self, url: &str, ancestor: &str, descendant: &str) -> Result<bool> {
        // Only a store has history; nothing else fetches it for a warning.
        if self.store(url) != Store::Repo {
            return Err(unavailable("no history on this machine"));
        }
        let path = self.path(url);
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

    fn unknown(&self, url: &str) {
        let path = self.path(url);
        if !self.online_for(&path) {
            self.missed(&path);
        }
    }

    fn local_branch(&self, url: &str, branch: &str) -> Option<String> {
        if self.store(url) != Store::Repo {
            return None;
        }
        let path = self.repo_path(url);
        if !is_store(&path) {
            return None;
        }
        crate::git::resolve_ref(&path, &format!("refs/heads/{}^{{commit}}", branch))
    }

    fn prepare(&self, wanted: &[String]) {
        // Refs of every repository, fetched side by side: the network wait
        // is what resolution costs, and it is per repository.
        let mut todo: BTreeMap<PathBuf, String> = BTreeMap::new();
        {
            let known = self.refs.lock().unwrap();
            for url in wanted {
                let path = self.path(url);
                if !known.contains_key(&path) {
                    todo.insert(path, url.clone());
                }
            }
        }
        if todo.len() < 2 {
            return;
        }
        std::thread::scope(|scope| {
            for url in todo.values() {
                scope.spawn(move || {
                    let _ = self.refs(url);
                });
            }
        });
    }
}

/// Where the workspace keeps its light stores: in its git directory, so they
/// never show up as untracked files.
fn local_dir(config_root: &Path) -> PathBuf {
    crate::git::git_path(config_root, "gitscale/resolve")
        .unwrap_or_else(|| config_root.join(".git/gitscale/resolve"))
}

fn is_store(path: &Path) -> bool {
    path.join("HEAD").is_file()
}

/// A store's view of its remote: the remote's branches as it last fetched
/// them, its tags, each peeled to its commit, and the default branch
/// `refs/remotes/origin/HEAD` names. The store's own branches are not the
/// remote's, and are not listed.
fn repo_refs(path: &Path) -> Result<Refs> {
    let dir = path.to_string_lossy().to_string();
    let listed = crate::git::run_git(
        &[
            "-C",
            &dir,
            "for-each-ref",
            "--format=%(refname)%09%(objectname)%09%(*objectname)",
            "refs/remotes/origin",
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
        if let Some(branch) = name.strip_prefix("refs/remotes/origin/") {
            if branch != "HEAD" {
                refs.branches.insert(branch.to_string(), commit);
            }
        } else if let Some(tag) = name.strip_prefix("refs/tags/") {
            refs.tags.insert(tag.to_string(), commit);
        }
    }
    refs.default_branch = crate::git::query(
        path,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .and_then(|b| b.strip_prefix("origin/").map(str::to_string))
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
    fn config_if_at(&self, directory: &str, commit: &str) -> Option<Option<String>> {
        let dest = self.root.join(directory);
        let at = if crate::git::is_checkout(&dest) {
            crate::git::resolve_ref(&dest, "HEAD").as_deref() == Some(commit)
        } else {
            crate::artefact::installed(&self.root, directory).is_some_and(|m| m.commit == commit)
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

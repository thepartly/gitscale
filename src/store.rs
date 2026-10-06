//! What a workspace root keeps for its dependencies, in its own git
//! directory: one bare clone per dependency URL, every checkout of it a
//! worktree, and the artefact images its checkouts were installed from.
//!
//! The directory is the root repository's *common* git dir — `root/.git` for
//! a plain clone, the bare repository for a root that is itself a set of
//! worktrees — so every worktree of the root shares one store per dependency:
//! a branch committed in one worktree's checkout is visible in every other,
//! one fetch updates them all, and nothing is downloaded twice. Deleting the
//! root deletes everything; nothing lives outside it.
//!
//! ```text
//! <common>/gitscale/repos/<name>.git   bare, full history, origin = the remote
//! <common>/gitscale/images/<name>/     one OCI image layout per artefact repository
//! <common>/gitscale/locks/             one advisory lock per store
//! ```
//!
//! A store keeps the remote's branches as `refs/remotes/origin/*` and its own
//! as `refs/heads/*`: a fetch never touches the branches checkouts develop
//! on. Git's own gc runs in each one as it would in any repository, since git
//! knows every worktree a store has.
//!
//! CI keeps no store: its checkouts are copies of exact commits, served from
//! the per-user cache — see [`crate::cache`]. The image store and the locks
//! are shared with it, which is why they live here.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// The stores' directory inside a common git dir.
const GITSCALE: &str = "gitscale";
const REPOS: &str = "repos";
const IMAGES: &str = "images";
const LOCKS: &str = "locks";
/// Touched when the image store was last pruned, so `pull` prunes at most
/// once a day.
const PRUNED: &str = "gitscale-pruned";
/// Touched on every use of an image entry, so pruning can tell live entries
/// from dead.
pub(crate) const LAST_USED: &str = "gitscale-last-used";
/// One marker per release an image entry holds, touched whenever that
/// release's image is used.
pub(crate) const PINS: &str = "gitscale-pins";

/// The directory name holding `url`'s store.
///
/// A normalized URL is `host/owner/repo`, but only when it parses as one —
/// otherwise it is the URL itself, which may be absolute or hold anything a
/// filesystem would rather not see. So the readable part is flattened into a
/// single component and a digest of the canonical form is appended: two
/// repositories that differ only where the flattening erased the difference
/// still get their own store, and nothing can escape the directory.
pub fn entry_name(url: &str) -> String {
    let canonical = crate::urls::normalize(url);
    let mut slug = String::with_capacity(canonical.len());
    let mut last_dash = true; // also trims a leading dash
    for c in canonical.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            slug.push(c);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches(['-', '.']);
    // Long enough to stay recognisable, short enough to keep the path sane on
    // a URL that is mostly filesystem path.
    let tail: String = match slug.char_indices().nth_back(47) {
        Some((start, _)) => slug[start..].to_string(),
        None => slug.to_string(),
    };
    let digest = digest12(&canonical);
    if tail.is_empty() {
        format!("{}.git", digest)
    } else {
        format!("{}-{}.git", tail, digest)
    }
}

/// [`entry_name`] without the `.git` suffix, for an entry that is an image
/// layout rather than a repository.
pub fn image_entry_name(url: &str) -> String {
    let name = entry_name(url);
    name.strip_suffix(".git").unwrap_or(&name).to_string()
}

fn digest12(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())[..12].to_string()
}

/// The root's stores. Constructed once per command and shared across the
/// threads that work on individual checkouts.
#[derive(Debug, Clone)]
pub struct Stores {
    root: PathBuf,
    /// Stores already fetched by this command: resolution fetches each one
    /// before the checkout built from it, and a second fetch would only ask
    /// the remote the same question again.
    fetched: Arc<Mutex<HashSet<PathBuf>>>,
}

impl Stores {
    /// The stores of the workspace whose root repository is at
    /// `config_root`.
    pub fn open(config_root: &Path) -> Result<Stores> {
        let common = crate::git::common_dir(config_root)
            .with_context(|| format!("{} is not in a git repository", config_root.display()))?;
        Ok(Stores {
            root: common.join(GITSCALE),
            fetched: Default::default(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where `url`'s store is, whether or not it exists yet.
    pub fn repo_path(&self, url: &str) -> PathBuf {
        self.root.join(REPOS).join(entry_name(url))
    }

    pub fn images(&self) -> ImageStore {
        ImageStore::new(self.root.join(IMAGES), self.root.join(LOCKS))
    }

    /// `url`'s store, created if it is not there, fetched from the remote
    /// once per command.
    pub fn update(&self, url: &str) -> Result<PathBuf> {
        let path = self.repo_path(url);
        if self.fetched.lock().unwrap().contains(&path) && is_repository(&path) {
            return Ok(path);
        }
        let _lock = Lock::acquire(&self.root.join(LOCKS), &path)?;
        ensure_store(&path, url)?;
        git_in(&path, &["fetch", "--prune", "--quiet", "origin"], true)
            .with_context(|| format!("cannot fetch {}", url))?;
        // The remote's default branch, which a fetch does not carry: asked
        // once, and kept as `refs/remotes/origin/HEAD` for every offline read.
        if crate::git::resolve_ref(&path, "refs/remotes/origin/HEAD").is_none() {
            let _ = git_in(&path, &["remote", "set-head", "origin", "--auto"], false);
        }
        self.fetched.lock().unwrap().insert(path.clone());
        Ok(path)
    }

    /// `url`'s store when it already holds `commit`; otherwise fetched, as
    /// [`Stores::update`] does. For a placement that fetches only what it
    /// lacks.
    pub fn ready(&self, url: &str, commit: &str) -> Result<PathBuf> {
        let path = self.repo_path(url);
        if is_repository(&path) && crate::git::ref_exists(&path, &format!("{}^{{commit}}", commit))
        {
            return Ok(path);
        }
        self.update(url)
    }

    /// Fetch the store at `path` from its own `origin`, once per command:
    /// for a store no entry asks for any more.
    pub fn update_path(&self, path: &Path) -> Result<()> {
        if self.fetched.lock().unwrap().contains(path) {
            return Ok(());
        }
        let _lock = Lock::acquire(&self.root.join(LOCKS), path)?;
        git_in(path, &["fetch", "--prune", "--quiet", "origin"], true)
            .with_context(|| format!("cannot fetch {}", path.display()))?;
        self.fetched.lock().unwrap().insert(path.to_path_buf());
        Ok(())
    }

    /// Every store this root has.
    pub fn all(&self) -> Vec<PathBuf> {
        let Ok(listing) = fs::read_dir(self.root.join(REPOS)) else {
            return Vec::new();
        };
        let mut stores: Vec<PathBuf> = listing
            .flatten()
            .map(|e| e.path())
            .filter(|p| is_repository(p))
            .collect();
        stores.sort();
        stores
    }

    /// Mend, then forget, the worktrees of every store, before anything
    /// creates one or puts one on a branch: `pull`, `develop` and `clean`.
    ///
    /// Mending first: moving the root moves its stores with it, and every
    /// link between a store and the checkouts of this root breaks at both
    /// ends — which a prune would take for checkouts that are gone. So each
    /// checkout this root recorded whose link points nowhere is repaired from
    /// the store its link names, found by that name in this root.
    ///
    /// Then forgetting the worktrees whose directories are gone — a root
    /// worktree removed with its checkouts in it, or deleted by hand. Not
    /// optional: git keeps such a worktree's branch checked out until it is
    /// pruned, and refuses that branch to every other worktree, so a topic
    /// developed in a deleted root worktree could not be developed anywhere
    /// else. Git's own gc prunes them only after three months.
    pub fn tidy(&self, config_root: &Path) {
        // Every worktree of the root has checkouts of these stores, not only
        // this one.
        for root in root_worktrees(config_root) {
            for directory in crate::ledger::git_checkouts(&root) {
                self.repair_moved(&root.join(directory));
            }
        }
        for store in self.all() {
            let _ = git_in(&store, &["worktree", "prune"], false);
        }
    }

    /// Repair the checkout at `dest` if its link names a store of this root
    /// that is no longer where the link says.
    fn repair_moved(&self, dest: &Path) {
        let Ok(text) = fs::read_to_string(dest.join(".git")) else {
            return;
        };
        let Some(link) = text.strip_prefix("gitdir:").map(str::trim) else {
            return;
        };
        let link = dest.join(link);
        if link.exists() {
            return;
        }
        // `…/gitscale/repos/<store>/worktrees/<id>`: the store's name is what
        // survives the move.
        let parts: Vec<String> = link
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let Some(at) = parts
            .windows(3)
            .position(|w| w[0] == REPOS && w[2] == "worktrees")
        else {
            return;
        };
        let store = self.root.join(REPOS).join(&parts[at + 1]);
        if is_repository(&store) {
            let dest_str = dest.to_string_lossy().to_string();
            let _ = git_in(&store, &["worktree", "repair", &dest_str], false);
        }
    }

    /// `git gc` in every store: what `gitscale clean --gc` asks for.
    pub fn gc(&self) -> Result<usize> {
        let stores = self.all();
        for store in &stores {
            git_in(store, &["gc", "--quiet"], true)
                .with_context(|| format!("cannot gc {}", store.display()))?;
        }
        Ok(stores.len())
    }

    /// Drop images nothing has used within `keep`, at most once a day: the
    /// step `pull` ends with. `Ok(None)` when it was done less than a day ago.
    pub fn prune_images_daily(&self, keep: Duration) -> Result<Option<Pruned>> {
        let marker = self.root.join(IMAGES).join(PRUNED);
        let day = SystemTime::now()
            .checked_sub(Duration::from_secs(86_400))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if !older_than(&marker, day) {
            return Ok(None);
        }
        let pruned = self.images().prune(keep)?;
        touch(&marker);
        Ok(Some(pruned))
    }
}

/// Where one command's checkouts and images come from: the root's stores on
/// a developer machine, the per-user cache in CI — worked out once per
/// command.
#[derive(Debug, Clone)]
pub struct Sources {
    /// `None` in CI, where checkouts are copies and no store is kept.
    pub stores: Option<Stores>,
    /// The CI cache: `None` outside CI, and with `--no-cache`.
    pub cache: Option<crate::cache::Cache>,
}

impl Sources {
    pub fn new(config_root: &Path, no_cache: bool) -> Result<Sources> {
        let ci = crate::git::is_ci();
        Ok(Sources {
            stores: if ci {
                None
            } else {
                Some(Stores::open(config_root)?)
            },
            cache: crate::cache::Cache::for_ci(no_cache),
        })
    }

    /// Where artefact images are kept: the root's image store, or the CI
    /// cache's. `None` in CI with the cache off: images are downloaded for the
    /// one install and dropped.
    pub fn images(&self) -> Option<ImageStore> {
        match (&self.stores, &self.cache) {
            (Some(stores), _) => Some(stores.images()),
            (None, Some(cache)) => Some(cache.images()),
            (None, None) => None,
        }
    }
}

/// Every worktree of the root repository at `config_root` that is on disk,
/// this one included.
fn root_worktrees(config_root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> =
        crate::git::query(config_root, &["worktree", "list", "--porcelain"])
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.strip_prefix("worktree "))
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .collect();
    if found.is_empty() {
        found.push(config_root.to_path_buf());
    }
    found
}

/// Whether `path` holds a repository, as opposed to a directory that is
/// missing, empty, or half-created.
pub fn is_repository(path: &Path) -> bool {
    path.join("HEAD").is_file()
}

/// Create the bare store for `url` at `path` if it is not there, and keep its
/// `origin` pointing at `url`.
fn ensure_store(path: &Path, url: &str) -> Result<()> {
    if is_repository(path) {
        crate::git::set_origin(path, url)?;
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("a store has no parent directory"))?;
    fs::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;
    // Built under another name and moved into place: an interrupted creation
    // must not leave a directory that later runs mistake for a usable store.
    let staging = path.with_extension("staging");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).with_context(|| format!("cannot create {}", staging.display()))?;
    git_in(&staging, &["init", "--bare", "--quiet"], true)?;
    git_in(&staging, &["remote", "add", "origin", url], true)?;
    // Not `--mirror`: that maps every remote ref onto a local one, and a fetch
    // would then reset the branches checkouts are developed on — and bring
    // every merge request's refs along.
    git_in(
        &staging,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
        true,
    )?;
    git_in(
        &staging,
        &[
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/tags/*:refs/tags/*",
        ],
        true,
    )?;
    // A bare repository keeps no reflogs unless told to; its worktrees'
    // branches are somebody's work.
    git_in(&staging, &["config", "core.logAllRefUpdates", "true"], true)?;
    // Relative links between the store and its worktrees: moving the whole
    // root then breaks nothing. Older git writes absolute ones, which `pull`
    // repairs after a move.
    if crate::git::supports_relative_worktrees() {
        git_in(
            &staging,
            &["config", "worktree.useRelativePaths", "true"],
            true,
        )?;
    }
    fs::rename(&staging, path)
        .with_context(|| format!("cannot move the new store into {}", path.display()))?;
    Ok(())
}

/// Run git inside a store.
fn git_in(store: &Path, args: &[&str], check: bool) -> Result<std::process::Output> {
    let dir = store.to_string_lossy().to_string();
    let mut full: Vec<&str> = vec!["-C", &dir];
    full.extend_from_slice(args);
    crate::git::run_git(&full, None, check)
}

// ---------------------------------------------------------------------------
// Worktrees
// ---------------------------------------------------------------------------

/// Make `dest` a worktree of `store`, detached at `commit`. An empty
/// directory there is taken over; anything else is refused by git.
pub fn add_worktree(store: &Path, dest: &Path, commit: &str) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    if dest.is_dir() && crate::artefact::is_empty_dir(dest) {
        fs::remove_dir(dest).with_context(|| format!("cannot remove {}", dest.display()))?;
    }
    let dest_str = dest.to_string_lossy().to_string();
    git_in(
        store,
        &["worktree", "add", "--quiet", "--detach", &dest_str, commit],
        true,
    )?;
    Ok(())
}

/// What the checkout at `dest` is, as far as `store` is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Worktree {
    /// A worktree of `store`.
    Ours,
    /// A worktree of `store` whose links broke because something moved, and
    /// that `repair` has just mended.
    Repaired,
    /// Something gitscale did not make: a clone of its own, a worktree of
    /// another repository, or one whose store is gone.
    Foreign,
}

/// Whether `dest` is a worktree of `store`, mending the links between them
/// when a move broke them.
///
/// A worktree's `.git` file names the store's record of it, and that record
/// names the worktree back. Moving the root (or, with git older than 2.48,
/// anything) breaks one end or both; `git worktree repair` rewrites them,
/// which is safe exactly when both ends are known — and here they are.
pub fn identify(store: &Path, dest: &Path) -> Worktree {
    if is_worktree_of(store, dest) {
        return Worktree::Ours;
    }
    if !dest.join(".git").is_file() {
        return Worktree::Foreign;
    }
    let dest_str = dest.to_string_lossy().to_string();
    let _ = git_in(store, &["worktree", "repair", &dest_str], false);
    if is_worktree_of(store, dest) {
        Worktree::Repaired
    } else {
        Worktree::Foreign
    }
}

/// Whether `dest` is a worktree of `store` with both links intact, changing
/// nothing.
pub fn is_worktree_of(store: &Path, dest: &Path) -> bool {
    dest.join(".git").is_file()
        && crate::git::common_dir(dest).is_some_and(|common| same_path(&common, store))
        && links_agree(dest)
}

/// Whether the store's record of the worktree at `dest` points back at it.
fn links_agree(dest: &Path) -> bool {
    let Some(admin) = crate::git::query(dest, &["rev-parse", "--absolute-git-dir"]) else {
        return false;
    };
    let Ok(recorded) = fs::read_to_string(Path::new(&admin).join("gitdir")) else {
        return false;
    };
    let recorded = Path::new(admin.as_str()).join(recorded.trim());
    same_path(&recorded, &dest.join(".git"))
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

/// Artefact images, one OCI image layout per repository: the root's own on a
/// developer machine, the per-user cache's in CI. Checkouts get copies of
/// what they unpack, so nothing borrows from an entry and pruning can never
/// break one.
#[derive(Debug, Clone)]
pub struct ImageStore {
    root: PathBuf,
    locks: PathBuf,
}

/// What a prune removed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pruned {
    /// Entries removed whole.
    pub entries: usize,
    /// Commits' images dropped from entries that stayed.
    pub images: usize,
    pub freed: u64,
}

impl ImageStore {
    pub fn new(root: PathBuf, locks: PathBuf) -> Self {
        ImageStore { root, locks }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The entry for `url`.
    pub fn path(&self, url: &str) -> PathBuf {
        self.root.join(image_entry_name(url))
    }

    /// The entry's lock: held from download to unpacking, so a prune never
    /// deletes a blob in between, and N cold starts download each blob once.
    pub(crate) fn lock(&self, entry: &Path) -> Result<Lock> {
        Lock::acquire(&self.locks, entry)
    }

    /// Mark an entry, and the release just served from it, as wanted.
    pub(crate) fn touch(&self, entry: &Path, tag: &str) {
        touch(&entry.join(LAST_USED));
        touch(&entry.join(PINS).join(tag));
    }

    /// Every entry the store holds.
    pub fn entries(&self) -> Vec<PathBuf> {
        let Ok(listing) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut entries: Vec<PathBuf> = listing
            .flatten()
            .map(|e| e.path())
            .filter(|p| crate::oci_layout::Layout::exists(p))
            .collect();
        entries.sort();
        entries
    }

    /// When anything last used the release `tag` of `entry`.
    pub fn used(entry: &Path, tag: &str) -> Option<SystemTime> {
        modified(&entry.join(PINS).join(tag))
    }

    /// Drop every image nothing has used within `keep`, then every blob no
    /// remaining image needs; an entry left with nothing goes whole.
    pub fn prune(&self, keep: Duration) -> Result<Pruned> {
        let cutoff = SystemTime::now()
            .checked_sub(keep)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let mut pruned = Pruned::default();
        for entry in self.entries() {
            let _lock = self.lock(&entry)?;
            let before = dir_size(&entry);
            if older_than(&entry.join(LAST_USED), cutoff) {
                fs::remove_dir_all(&entry)
                    .with_context(|| format!("cannot remove {}", entry.display()))?;
                pruned.entries += 1;
                pruned.freed += before;
                continue;
            }
            let layout = crate::oci_layout::Layout::new(entry.clone());
            let cold: Vec<String> = layout
                .held()
                .into_iter()
                .map(|held| held.tag)
                .filter(|tag| older_than(&entry.join(PINS).join(tag), cutoff))
                .collect();
            layout.forget(&cold)?;
            for tag in &cold {
                let _ = fs::remove_file(entry.join(PINS).join(tag));
            }
            pruned.images += cold.len();
            layout.gc()?;
            pruned.freed += before.saturating_sub(dir_size(&entry));
        }
        Ok(pruned)
    }
}

// ---------------------------------------------------------------------------
// Locks and markers
// ---------------------------------------------------------------------------

/// An exclusive lock on one store or entry, released when the guard is
/// dropped — including when the process dies, which a lock file of our own
/// making would not be.
pub(crate) struct Lock {
    _file: fs::File,
}

impl Lock {
    /// The lock for `entry`, kept in `dir`.
    pub(crate) fn acquire(dir: &Path, entry: &Path) -> Result<Lock> {
        let name = entry
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "store".to_string());
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        let path = dir.join(format!("{}.lock", name));
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("cannot open the lock {}", path.display()))?;
        // Blocks until whoever holds it is done: that is the point, since the
        // alternative is N processes downloading the same repository at once.
        file.lock()
            .with_context(|| format!("cannot lock {}", path.display()))?;
        Ok(Lock { _file: file })
    }
}

pub(crate) fn touch(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, b"");
}

pub(crate) fn modified(marker: &Path) -> Option<SystemTime> {
    fs::metadata(marker).and_then(|m| m.modified()).ok()
}

/// True when `marker` was last touched before `cutoff` — or is not there at
/// all.
pub(crate) fn older_than(marker: &Path, cutoff: SystemTime) -> bool {
    match modified(marker) {
        Some(modified) => modified < cutoff,
        None => true,
    }
}

pub(crate) fn dir_size(path: &Path) -> u64 {
    let Ok(listing) = fs::read_dir(path) else {
        return 0;
    };
    listing
        .flatten()
        .map(|found| match found.file_type() {
            Ok(kind) if kind.is_dir() => dir_size(&found.path()),
            Ok(kind) if kind.is_file() => found.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

/// The period `[clean] keep_recent` gives, or the default.
pub fn keep_recent(configured: Option<&str>) -> Result<Duration> {
    crate::cache::parse_period(configured.unwrap_or(crate::config::DEFAULT_KEEP_RECENT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_forms_of_one_repo_share_a_store() {
        assert_eq!(
            entry_name("git@github.com:org/repo.git"),
            entry_name("https://github.com/ORG/repo")
        );
        assert!(entry_name("git@github.com:org/repo.git").starts_with("github.com-org-repo-"));
    }

    #[test]
    fn different_repos_never_share_one() {
        assert_ne!(
            entry_name("https://a.example/x/y"),
            entry_name("https://b.example/x/y")
        );
    }

    #[test]
    fn a_store_name_is_one_harmless_path_component() {
        // A local remote normalizes to an absolute path; its store must still
        // land inside the directory.
        let name = entry_name("/srv/mirrors/../../etc/repo.git");
        assert_eq!(Path::new(&name).components().count(), 1);
        assert!(!name.starts_with('.'), "{}", name);
        assert!(Path::new("/stores").join(&name).starts_with("/stores"));
    }

    #[test]
    fn an_image_entry_is_named_like_its_store() {
        let url = "https://github.com/org/repo";
        assert_eq!(format!("{}.git", image_entry_name(url)), entry_name(url));
    }

    #[test]
    fn markers_older_than_a_cutoff_and_missing_ones_count_as_old() {
        let dir = std::env::temp_dir().join(format!("gitscale-store-{}", std::process::id()));
        let marker = dir.join("m");
        assert!(older_than(&marker, SystemTime::now()));
        touch(&marker);
        assert!(!older_than(
            &marker,
            SystemTime::now() - Duration::from_secs(60)
        ));
        let _ = fs::remove_dir_all(&dir);
    }
}

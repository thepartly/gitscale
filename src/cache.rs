//! The object cache: one bare git repository per remote URL, kept in the
//! user's own data directory and borrowed from by every workspace on the
//! machine.
//!
//! One rule governs the direction: **the cache talks to the remote, the
//! workspace talks to the cache.** Every clone, fetch and pull updates the
//! cache entry first, then builds or updates the workspace from it. Nothing
//! flows back from a workspace, there is no timer and no TTL, and nothing
//! depends on anyone running a maintenance command.
//!
//! The cache is **per user**, and there is deliberately no system-wide scope
//! for it. A cache several users write through would give anything that
//! poisons one entry a machine-wide reach — the blast radius
//! `hook install --system` gets only after an explicit `--allow`, and
//! something on by default can never ask for. `dir` still points wherever the
//! user says; sharing that path with anyone else is their arrangement, not a
//! mode gitscale sets up.
//!
//! Two kinds of entry, because developer machines and CI want opposite things:
//!
//! * **mirror** — full history, updated from the remote before each workspace
//!   operation, consumed with `git clone --reference`. History to browse,
//!   objects shared across workspaces.
//! * **snapshot** — a shallow bare repo holding each pinned commit as
//!   `refs/heads/pin/<sha>`, consumed by an ordinary local clone. The depth-1
//!   checkout CI uses today, served from local disk. Git refuses a shallow
//!   repository as a `--reference`, which is why this one is copied rather
//!   than borrowed — and why deleting the entry under a running job is
//!   harmless.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::config::{CacheSettings, RepoEntry};
use crate::share::{Pinned, Reference, Source};

/// Mirror entries: `<cache>/mirror/<entry>.git`.
const MIRRORS: &str = "mirror";
/// Snapshot entries: `<cache>/snapshots/<entry>.git`.
const SNAPSHOTS: &str = "snapshots";
/// Advisory locks, one file per entry name.
const LOCKS: &str = "locks";
/// Touched on every hit, so `cache compact` can tell live entries from dead.
const LAST_USED: &str = "gitscale-last-used";
/// One marker per pinned commit, touched on every hit.
const PINS: &str = "gitscale-pins";

/// Where the cache lives, given the configured `dir` (empty for the default).
///
/// Every branch lands somewhere the invoking user owns. There is no fallback
/// to a machine-wide location: a cache that cannot be per-user is not built at
/// all, and gitscale talks to the remote instead.
pub fn resolve_dir(configured: &str) -> Option<PathBuf> {
    if !configured.is_empty() {
        return Some(expand_tilde(configured));
    }
    if let Some(dir) = var("GITSCALE_CACHE_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(data) = var("XDG_DATA_HOME") {
        return Some(PathBuf::from(data).join("gitscale"));
    }
    var("HOME").map(|home| PathBuf::from(home).join(".local/share/gitscale"))
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn expand_tilde(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => match var("HOME") {
            Some(home) => PathBuf::from(home).join(rest),
            None => PathBuf::from(path),
        },
        None => PathBuf::from(path),
    }
}

/// The directory name holding `url`'s entry.
///
/// A normalized URL is `host/owner/repo`, but only when it parses as one —
/// otherwise it is the URL itself, which may be absolute or hold anything a
/// filesystem would rather not see. So the readable part is flattened into a
/// single component and a digest of the canonical form is appended: two
/// repositories that differ only where the flattening erased the difference
/// still get their own entry, and nothing can escape the cache directory.
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

fn digest12(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())[..12].to_string()
}

/// An open cache. Constructed once per command and shared across the threads
/// that operate on individual repositories.
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
    dissociate: bool,
}

impl Cache {
    /// The cache `settings` describe, or `None` when it is switched off or
    /// this user has no data directory to keep one in.
    pub fn open(settings: &CacheSettings) -> Option<Self> {
        if !settings.enabled {
            return None;
        }
        Some(Self {
            root: resolve_dir(&settings.dir)?,
            dissociate: settings.dissociate,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn mirror_path(&self, url: &str) -> PathBuf {
        self.root.join(MIRRORS).join(entry_name(url))
    }

    pub fn snapshot_path(&self, url: &str) -> PathBuf {
        self.root.join(SNAPSHOTS).join(entry_name(url))
    }

    /// The mirror entry for `url`, brought up to date from the remote.
    ///
    /// This is the one network call in a cache-first operation: every step
    /// after it reads from local disk. N workspaces share it, which is the
    /// whole saving, and why the entry is updated even when the workspace
    /// could have fetched for itself.
    pub fn mirror(&self, url: &str) -> Result<PathBuf> {
        let path = self.mirror_path(url);
        let _lock = self.lock(&path)?;
        ensure_entry(&path, url)?;
        // `remote update -p` rather than `fetch`: the entry is configured as a
        // mirror, so this is the call that keeps every ref — and only the refs
        // the remote still has — in step with it.
        git_in(&path, &["remote", "update", "-p"], true)
            .with_context(|| format!("cannot update the cache entry for {}", url))?;
        touch(&path.join(LAST_USED));
        Ok(path)
    }

    /// The snapshot entry's copy of the commit `revision` names, adding it if
    /// the entry does not have it yet.
    ///
    /// `Ok(None)` means this revision cannot be cached — an empty revision, a
    /// ref the remote does not advertise, or a bare SHA it refuses to serve —
    /// leaving the caller to fetch it the direct way, as gitscale always has.
    pub fn pin(&self, url: &str, revision: &str) -> Result<Option<Pinned>> {
        let Some((sha, detach)) = resolve_pin(url, revision)? else {
            return Ok(None);
        };
        let path = self.snapshot_path(url);
        let _lock = self.lock(&path)?;
        ensure_entry(&path, url)?;

        let reference = format!("refs/heads/pin/{}", sha);
        let have = git_in(
            &path,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{}^{{commit}}", reference),
            ],
            false,
        )?;
        if !have.status.success() {
            // One commit, no history: everything this pin shares with a pin
            // already in the entry is already here.
            let fetched = git_in(&path, &["fetch", "--depth", "1", "origin", &sha], false)?;
            if !fetched.status.success() {
                return Ok(None);
            }
            git_in(&path, &["update-ref", &reference, "FETCH_HEAD"], true)?;
        }
        touch(&path.join(LAST_USED));
        touch(&path.join(PINS).join(&sha));
        Ok(Some(Pinned {
            entry: path,
            // `clone --branch` does not look outside `refs/heads`, which is
            // why pins live there and are named this way.
            reference: format!("pin/{}", sha),
            sha,
            detach,
        }))
    }

    /// Seed an entry from a repository already on this machine and point that
    /// repository at it, reclaiming the objects it no longer needs to own.
    ///
    /// Local, no network. `repack -a -d -l` deletes the repository's own
    /// objects, so a repository that stood on its own comes out depending on
    /// the cache — recoverable with `cache repair`, but the user's call, which
    /// is why nothing here runs unless `[cache] adopt_root` says so.
    pub fn adopt(&self, repo: &Path, url: &str) -> Result<bool> {
        // A repository that already borrows is already thin, and whatever it
        // borrows from is not necessarily this cache. Overwriting that pointer
        // and then repacking with `-l` would leave it unable to read objects
        // it never owned — so anything with an alternate is left alone.
        if !alternates_of(repo).is_empty() {
            return Ok(false);
        }
        let path = self.mirror_path(url);
        let _lock = self.lock(&path)?;
        ensure_entry(&path, url)?;
        let repo_str = repo.to_string_lossy().to_string();
        // The repository's remote-tracking refs are what the remote actually
        // has; its local branches are the user's and have no business in an
        // entry every other workspace reads.
        git_in(
            &path,
            &[
                "fetch",
                &repo_str,
                "+refs/remotes/origin/*:refs/heads/*",
                "^refs/remotes/origin/HEAD",
            ],
            true,
        )?;
        let alternates = crate::git::git_path(repo, "objects/info/alternates")
            .context("cannot locate the object store to relink")?;
        if let Some(parent) = alternates.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&alternates, format!("{}\n", path.join("objects").display()))
            .with_context(|| format!("cannot write {}", alternates.display()))?;
        crate::git::repack_local(repo)?;
        touch(&path.join(LAST_USED));
        Ok(true)
    }

    /// Re-create the entry `repo` borrows from, after something deleted it.
    ///
    /// Returns `None` when `repo` does not borrow from this cache at all,
    /// `Some(false)` when the entry it borrows from is still there.
    pub fn repair(&self, repo: &Path, url: &str) -> Result<Option<bool>> {
        let Some(entry) = borrowed_from(repo, &self.root) else {
            return Ok(None);
        };
        if is_entry(&entry) {
            return Ok(Some(false));
        }
        self.mirror(url)?;
        Ok(Some(true))
    }

    /// Every entry in the cache, mirrors and snapshots alike.
    pub fn entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        for (kind, dir) in [(Kind::Mirror, MIRRORS), (Kind::Snapshot, SNAPSHOTS)] {
            let Ok(listing) = fs::read_dir(self.root.join(dir)) else {
                continue;
            };
            for found in listing.flatten() {
                let path = found.path();
                if is_entry(&path) {
                    entries.push(Entry { kind, path });
                }
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries
    }

    /// What the cache costs on disk right now — the number `gitscale status`
    /// reports, so a full disk is something the user was told about rather
    /// than something they discover.
    pub fn usage(&self) -> Usage {
        let entries = self.entries();
        Usage {
            bytes: entries.iter().map(|e| dir_size(&e.path)).sum(),
            entries: entries.len(),
        }
    }

    /// What each entry costs, when it was last wanted, and which revisions it
    /// is holding.
    ///
    /// The revisions are always read: they are one `for-each-ref` per entry,
    /// and a count nobody can see for half the rows is worse than no column.
    /// Whether to *print* them all is the caller's business.
    pub fn stats(&self) -> Vec<EntryStats> {
        self.entries()
            .into_iter()
            .map(|entry| {
                let held = self.held_by(&entry);
                EntryStats {
                    bytes: dir_size(&entry.path),
                    last_used: modified(&entry.path.join(LAST_USED)),
                    revisions: held,
                    kind: entry.kind,
                    path: entry.path,
                }
            })
            .collect()
    }

    /// The revisions an entry holds: a snapshot's pins, or every ref of a
    /// mirror — which is every branch and tag the remote has.
    fn held_by(&self, entry: &Entry) -> Vec<Revision> {
        let mut args = vec!["for-each-ref", "--format=%(refname:short)"];
        if entry.kind == Kind::Snapshot {
            args.push("refs/heads/pin/");
        }
        let Ok(listed) = git_in(&entry.path, &args, false) else {
            return Vec::new();
        };
        String::from_utf8_lossy(&listed.stdout)
            .lines()
            .map(|name| {
                let sha = name.strip_prefix("pin/").unwrap_or(name);
                Revision {
                    // Each pin carries its own marker, so an entry in daily use
                    // can still show which of its commits have gone cold.
                    last_used: modified(&entry.path.join(PINS).join(sha)),
                    name: name.to_string(),
                }
            })
            .collect()
    }

    /// Repack every entry and drop what nothing has used for `keep`.
    pub fn compact(&self, keep: Duration) -> Result<Compacted> {
        let cutoff = SystemTime::now()
            .checked_sub(keep)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let mut report = Compacted::default();
        for entry in self.entries() {
            let _lock = self.lock(&entry.path)?;
            if older_than(&entry.path.join(LAST_USED), cutoff) {
                report.freed += dir_size(&entry.path);
                fs::remove_dir_all(&entry.path)
                    .with_context(|| format!("cannot remove {}", entry.path.display()))?;
                report.entries += 1;
                continue;
            }
            let before = dir_size(&entry.path);
            if entry.kind == Kind::Snapshot {
                report.pins += self.evict_pins(&entry.path, cutoff)?;
                // Nothing borrows from a snapshot — a job copies what it takes
                // — so pruning here cannot leave anyone unable to read.
                git_in(&entry.path, &["gc", "--prune=now", "--quiet"], false)?;
            } else {
                // A mirror has borrowers, and `repack -d` is the one operation
                // that can delete objects a live one still needs. Repack, keep
                // everything.
                git_in(&entry.path, &["gc", "--no-prune", "--quiet"], false)?;
            }
            report.freed += before.saturating_sub(dir_size(&entry.path));
        }
        Ok(report)
    }

    /// Drop pin refs nothing has taken since `cutoff`.
    fn evict_pins(&self, entry: &Path, cutoff: SystemTime) -> Result<usize> {
        let listed = git_in(
            entry,
            &["for-each-ref", "--format=%(refname)", "refs/heads/pin/"],
            false,
        )?;
        if !listed.status.success() {
            return Ok(0);
        }
        let refs: Vec<String> = String::from_utf8_lossy(&listed.stdout)
            .lines()
            .map(str::to_string)
            .collect();
        let mut dropped = 0;
        for reference in refs {
            let Some(sha) = reference.strip_prefix("refs/heads/pin/") else {
                continue;
            };
            let marker = entry.join(PINS).join(sha);
            if !older_than(&marker, cutoff) {
                continue;
            }
            git_in(entry, &["update-ref", "-d", &reference], true)?;
            let _ = fs::remove_file(&marker);
            dropped += 1;
        }
        Ok(dropped)
    }

    /// Take the entry's lock. Held for as long as the returned guard lives, so
    /// N cold starts at once cost one download rather than N.
    fn lock(&self, entry: &Path) -> Result<Lock> {
        let name = entry
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "cache".to_string());
        let dir = self.root.join(LOCKS);
        fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
        Lock::acquire(&dir.join(format!("{}.lock", name)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Mirror,
    Snapshot,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: Kind,
    pub path: PathBuf,
}

/// One entry, as `gitscale cache status` reports it.
#[derive(Debug, Clone)]
pub struct EntryStats {
    pub kind: Kind,
    pub path: PathBuf,
    pub bytes: u64,
    pub last_used: Option<SystemTime>,
    /// Every revision the entry holds: a snapshot's pins, a mirror's refs.
    pub revisions: Vec<Revision>,
}

/// A revision an entry holds.
#[derive(Debug, Clone)]
pub struct Revision {
    pub name: String,
    pub last_used: Option<SystemTime>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub entries: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Compacted {
    /// Entries removed whole.
    pub entries: usize,
    /// Pin refs dropped from entries that stayed.
    pub pins: usize,
    pub freed: u64,
}

/// Work out where `entry`'s checkout should come from, updating the cache
/// entry it will read on the way — the "cache first" step.
///
/// Everything degrades to the direct remote: a cache that is off, a revision
/// no snapshot can hold, an entry that will not update. A stale or missing
/// cache changes how many bytes cross the wire, never which commit you land
/// on.
pub fn source_for(
    cache: Option<&Cache>,
    workspace: Option<&Path>,
    entry: &RepoEntry,
    share_dissociate: bool,
    ci: bool,
    verbose: bool,
) -> Source {
    // An artefact is an unpacked archive with no object store — there is
    // nothing for a cache entry to hold.
    if entry.is_artefact() {
        return Source::default();
    }
    let borrowed = workspace.and_then(|w| crate::share::reference_for(w, entry, share_dissociate));
    let direct = |reference| Source::remote(reference, ci || entry.is_readonly());

    let Some(cache) = cache else {
        return direct(borrowed);
    };
    let url = crate::git::remote_url(entry);

    // Selection is by environment, not by revision kind: CI takes snapshots,
    // developer machines take mirrors.
    if ci {
        return match cache.pin(&url, &entry.revision) {
            Ok(Some(pinned)) => Source {
                reference: None,
                local: Some(pinned.entry.clone()),
                pinned: Some(pinned),
                shallow: true,
            },
            Ok(None) => direct(borrowed),
            Err(e) => {
                warn(verbose, &entry.directory, &e);
                direct(borrowed)
            }
        };
    }

    match cache.mirror(&url) {
        Ok(path) => Source {
            // The source workspace still wins when it has the repo; the cache
            // covers its misses. Either way the entry was just updated, so a
            // later workspace that falls back to it finds it current.
            reference: borrowed.or_else(|| {
                Some(Reference {
                    path: path.clone(),
                    dissociate: cache.dissociate,
                })
            }),
            local: Some(path),
            pinned: None,
            // A mirror must not force shallow: the workspace borrows from it,
            // so shallow buys nothing and borrows less cleanly.
            shallow: false,
        },
        Err(e) => {
            warn(verbose, &entry.directory, &e);
            direct(borrowed)
        }
    }
}

/// Say what the cache could not do, then get on with the operation without it.
/// Written straight to stderr so it appears beside interactive progress bars
/// rather than after them.
fn warn(verbose: bool, directory: &str, error: &anyhow::Error) {
    if verbose {
        eprintln!("  cache {}: {} (using the remote)", directory, error);
    }
}

/// What the cache is doing for one checkout — the per-repo half of what
/// `gitscale status` reports, shown as its `CACHE` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheUse {
    /// Nothing: this repository has no entry, and the checkout owns its
    /// objects.
    Unused,
    /// The checkout owns its objects, but the cache holds an entry for the
    /// repository anyway — the shape of every checkout made before the cache
    /// existed. `--reference` is a decision taken when a repository is cloned,
    /// and nothing re-links a clone afterwards, so the objects simply sit in
    /// both places. The entry is still kept current, and still serves every
    /// other workspace on the machine.
    Copied,
    /// Borrowing from a mirror entry.
    Mirror,
    /// Built from a snapshot entry holding this exact commit. A job copies
    /// what it takes, so unlike the others this is not a link on disk — see
    /// [`served_by_snapshot`].
    Snapshot,
    /// Borrowing from a source workspace instead — that still wins when it has
    /// the repo, and the cache entry is kept current behind it.
    Workspace,
    /// Borrowing from something that is no longer there. The checkout cannot
    /// read its own history, and git will not say so until something tries to
    /// read an object: this is the state `gitscale cache repair` exists for.
    Broken,
}

impl CacheUse {
    pub fn label(self) -> &'static str {
        match self {
            CacheUse::Unused => "-",
            CacheUse::Copied => "copy",
            CacheUse::Mirror => "mirror",
            CacheUse::Snapshot => "snapshot",
            CacheUse::Workspace => "workspace",
            CacheUse::Broken => "broken",
        }
    }
}

/// What the cache is doing for the checkout at `repo`.
///
/// `cache_root` is the cache's location whether or not the cache is switched
/// on: `--no-cache` changes what the next command will do, not where the
/// objects a checkout already borrows happen to live.
pub fn cache_use(repo: &Path, url: &str, cache_root: Option<&Path>) -> CacheUse {
    let mut used = CacheUse::Unused;
    for store in alternates_of(repo) {
        if !store.is_dir() {
            return CacheUse::Broken;
        }
        used = match cache_root {
            Some(root) if store.starts_with(root) => CacheUse::Mirror,
            _ => CacheUse::Workspace,
        };
    }
    if used != CacheUse::Unused {
        return used;
    }
    if served_by_snapshot(repo, url, cache_root) {
        return CacheUse::Snapshot;
    }
    if has_entry(url, cache_root) {
        return CacheUse::Copied;
    }
    CacheUse::Unused
}

/// Whether the cache holds an entry of either kind for `url`.
fn has_entry(url: &str, cache_root: Option<&Path>) -> bool {
    let Some(root) = cache_root else {
        return false;
    };
    let name = entry_name(url);
    [MIRRORS, SNAPSHOTS]
        .iter()
        .any(|kind| is_entry(&root.join(kind).join(&name)))
}

/// Whether a snapshot entry holds the commit this checkout is on.
///
/// Snapshots are consumed by a local clone, which copies the objects and
/// leaves nothing behind pointing at the entry — so this is the honest claim
/// available: not "these objects came from there", but "that entry holds this
/// commit", which is what decides whether the next job pays for it again.
fn served_by_snapshot(repo: &Path, url: &str, cache_root: Option<&Path>) -> bool {
    let Some(root) = cache_root else {
        return false;
    };
    let entry = root.join(SNAPSHOTS).join(entry_name(url));
    if !is_entry(&entry) {
        return false;
    }
    let Ok(head) = crate::git::run_git(&["rev-parse", "HEAD"], Some(repo), false) else {
        return false;
    };
    if !head.status.success() {
        return false;
    }
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    git_in(
        &entry,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/pin/{}", head),
        ],
        false,
    )
    .map(|o| o.status.success())
    .unwrap_or(false)
}

/// The object stores `repo` borrows from, whatever they are.
fn alternates_of(repo: &Path) -> Vec<PathBuf> {
    let Some(path) = crate::git::git_path(repo, "objects/info/alternates") else {
        return Vec::new();
    };
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(PathBuf::from)
        .collect()
}

/// The cache entry `repo` borrows from, if it borrows from one inside `root`.
fn borrowed_from(repo: &Path, root: &Path) -> Option<PathBuf> {
    alternates_of(repo)
        .into_iter()
        .find(|objects| objects.starts_with(root))
        // `<entry>/objects` back to `<entry>`.
        .and_then(|objects| objects.parent().map(Path::to_path_buf))
}

/// True for a directory that holds a built cache entry, as opposed to one that
/// is missing, empty, or half-created.
fn is_entry(path: &Path) -> bool {
    path.join("HEAD").is_file()
}

/// Create the bare entry for `url` if it is not there, and keep its `origin`
/// pointing at the URL we would actually fetch from — which changes under CI,
/// where the same repository is reached over HTTPS with a job token.
fn ensure_entry(path: &Path, url: &str) -> Result<()> {
    if is_entry(path) {
        let current = git_in(path, &["remote", "get-url", "origin"], false)?;
        let args: &[&str] = if current.status.success() {
            if String::from_utf8_lossy(&current.stdout).trim() == url {
                return Ok(());
            }
            &["remote", "set-url", "origin"]
        } else {
            &["remote", "add", "--mirror=fetch", "origin"]
        };
        let mut full = args.to_vec();
        full.push(url);
        git_in(path, &full, true)?;
        return Ok(());
    }

    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cache entry has no parent directory"))?;
    fs::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;
    // Build under another name and rename into place: an interrupted creation
    // must not leave a directory that later runs mistake for a usable entry.
    let staging = path.with_extension("staging");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).with_context(|| format!("cannot create {}", staging.display()))?;
    git_in(&staging, &["init", "--bare", "--quiet"], true)?;
    git_in(
        &staging,
        &["remote", "add", "--mirror=fetch", "origin", url],
        true,
    )?;
    // `repack -d` in an entry can delete objects a live borrower still needs,
    // and nothing warns when it does. Only `cache compact` may repack, and it
    // knows which entries have borrowers.
    git_in(&staging, &["config", "gc.auto", "0"], true)?;
    fs::rename(&staging, path)
        .with_context(|| format!("cannot move the new entry into {}", path.display()))?;
    Ok(())
}

/// The commit `revision` names, and whether checking it out should detach.
///
/// A SHA needs no network at all. A branch or tag costs one `ls-remote` — a
/// ref advertisement, no objects — and if the head has not moved, the SHA it
/// returns is already in the entry and nothing further transfers.
fn resolve_pin(url: &str, revision: &str) -> Result<Option<(String, bool)>> {
    if revision.is_empty() {
        return Ok(None);
    }
    if crate::git::looks_like_sha(revision) {
        return Ok(Some((revision.to_string(), true)));
    }
    let listed = crate::git::run_git(&["ls-remote", url, revision], None, false)?;
    if !listed.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&listed.stdout);
    let (mut branch, mut peeled, mut tag) = (None, None, None);
    for line in text.lines() {
        let Some((sha, name)) = line.split_once('\t') else {
            continue;
        };
        if name.ends_with("^{}") {
            peeled = Some(sha);
        } else if name.starts_with("refs/heads/") {
            branch = Some(sha);
        } else if name.starts_with("refs/tags/") {
            tag = Some(sha);
        }
    }
    // A branch keeps its name through the checkout; a tag peels to the commit
    // it points at and leaves HEAD detached, exactly as `clone --branch` does.
    Ok(match (branch, peeled.or(tag)) {
        (Some(sha), _) => Some((sha.to_string(), false)),
        (None, Some(sha)) => Some((sha.to_string(), true)),
        (None, None) => None,
    })
}

/// Run git inside a bare cache entry.
fn git_in(entry: &Path, args: &[&str], check: bool) -> Result<std::process::Output> {
    let dir = entry.to_string_lossy().to_string();
    let mut full: Vec<&str> = vec!["-C", &dir];
    full.extend_from_slice(args);
    crate::git::run_git(&full, None, check)
}

fn touch(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, b"");
}

fn modified(marker: &Path) -> Option<SystemTime> {
    fs::metadata(marker).and_then(|m| m.modified()).ok()
}

/// True when `marker` was last touched before `cutoff` — or is not there at
/// all, which is what an entry built before markers existed looks like.
fn older_than(marker: &Path, cutoff: SystemTime) -> bool {
    match fs::metadata(marker).and_then(|m| m.modified()) {
        Ok(modified) => modified < cutoff,
        Err(_) => true,
    }
}

fn dir_size(path: &Path) -> u64 {
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

/// Render a byte count the way `du -h` would.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", size, UNITS[unit])
    }
}

/// "1 entry", "3 entries". Four lines, because these strings are what the
/// user reads.
pub fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{} {}", count, if count == 1 { one } else { many })
}

/// How long ago, in the roughest unit that still says something: this is read
/// beside an eviction period, not used to compute one.
pub fn ago(when: Option<SystemTime>) -> String {
    let Some(when) = when else {
        return "unknown".to_string();
    };
    let Ok(elapsed) = when.elapsed() else {
        // A clock that moved, or a marker written by a faster machine.
        return "just now".to_string();
    };
    let seconds = elapsed.as_secs();
    let (count, one, many) = match seconds {
        0..=59 => return "just now".to_string(),
        60..=3_599 => (seconds / 60, "minute", "minutes"),
        3_600..=86_399 => (seconds / 3_600, "hour", "hours"),
        86_400..=604_799 => (seconds / 86_400, "day", "days"),
        604_800..=2_591_999 => (seconds / 604_800, "week", "weeks"),
        _ => (seconds / 2_592_000, "month", "months"),
    };
    format!("{} ago", plural(count as usize, one, many))
}

/// Parse a `--keep-recent` period: a count and a unit, as in `1month`, `30d`
/// or `2 weeks`.
pub fn parse_period(text: &str) -> Result<Duration> {
    let text = text.trim();
    let split = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (count, unit) = text.split_at(split);
    let count: u64 = count
        .parse()
        .map_err(|_| anyhow::anyhow!("'{}' does not start with a number", text))?;
    let unit = unit.trim().trim_end_matches('s');
    let seconds = match unit {
        "h" | "hour" => 3_600,
        "d" | "day" | "" => 86_400,
        "w" | "week" => 7 * 86_400,
        "m" | "month" => 30 * 86_400,
        "y" | "year" => 365 * 86_400,
        other => bail!(
            "unknown period unit '{}' (expected hours, days, weeks, months or years)",
            other
        ),
    };
    Ok(Duration::from_secs(count * seconds))
}

/// An exclusive lock on one cache entry, released when the guard is dropped —
/// including when the process dies, which a lock file of our own making would
/// not be.
struct Lock {
    _file: fs::File,
}

impl Lock {
    fn acquire(path: &Path) -> Result<Lock> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .with_context(|| format!("cannot open the cache lock {}", path.display()))?;
        // Blocks until whoever holds it is done: that is the point, since the
        // alternative is N processes downloading the same repository at once.
        file.lock()
            .with_context(|| format!("cannot lock {}", path.display()))?;
        Ok(Lock { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_forms_of_one_repo_share_an_entry() {
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
    fn an_entry_name_is_one_harmless_path_component() {
        // A local remote normalizes to an absolute path; the entry for it must
        // still land inside the cache directory.
        let name = entry_name("/srv/mirrors/../../etc/repo.git");
        assert_eq!(Path::new(&name).components().count(), 1);
        assert!(!name.starts_with('.'), "{}", name);
        assert!(Path::new("/cache").join(&name).starts_with("/cache"));
    }

    #[test]
    fn periods_parse_the_way_the_flag_spells_them() {
        assert_eq!(
            parse_period("1month").unwrap(),
            Duration::from_secs(30 * 86_400)
        );
        assert_eq!(
            parse_period("30d").unwrap(),
            Duration::from_secs(30 * 86_400)
        );
        assert_eq!(
            parse_period("2 weeks").unwrap(),
            Duration::from_secs(14 * 86_400)
        );
        assert_eq!(
            parse_period("12h").unwrap(),
            Duration::from_secs(12 * 3_600)
        );
        // A bare number is a count of days.
        assert_eq!(parse_period("7").unwrap(), Duration::from_secs(7 * 86_400));
        assert!(parse_period("soon").is_err());
        assert!(parse_period("3 fortnights").is_err());
    }

    #[test]
    fn counts_are_not_written_1_entries() {
        assert_eq!(plural(1, "entry", "entries"), "1 entry");
        assert_eq!(plural(0, "entry", "entries"), "0 entries");
        assert_eq!(plural(4, "pin", "pins"), "4 pins");
    }

    #[test]
    fn elapsed_time_reads_in_the_roughest_useful_unit() {
        let ago_by = |secs| ago(Some(SystemTime::now() - Duration::from_secs(secs)));
        assert_eq!(ago_by(5), "just now");
        assert_eq!(ago_by(90), "1 minute ago");
        assert_eq!(ago_by(3 * 3_600), "3 hours ago");
        assert_eq!(ago_by(2 * 86_400), "2 days ago");
        assert_eq!(ago_by(3 * 604_800), "3 weeks ago");
        assert_eq!(ago_by(90 * 86_400), "3 months ago");
        assert_eq!(ago(None), "unknown");
    }

    #[test]
    fn sizes_read_like_du() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KiB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MiB");
    }
}

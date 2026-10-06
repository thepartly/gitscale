//! The CI cache: exact revisions CI jobs asked for, kept in the invoking
//! user's own data directory so the next job on the same runner downloads
//! nothing.
//!
//! A developer machine keeps no cache: every dependency's store lives in the
//! root's own git directory — see [`crate::store`]. CI checkouts are depth-1
//! copies of exact commits, so what is worth keeping between jobs is exactly
//! those commits and the images artefacts were installed from, and
//! nothing in a job ever borrows from an entry: deleting one, or all of
//! them, costs a download and never breaks a checkout.
//!
//! * **snapshot** — a shallow bare repo holding each commit a job took as
//!   `refs/heads/pin/<sha>`, consumed by an ordinary local clone.
//! * **image** — an OCI image layout per artefact repository, blob by blob;
//!   checkouts get copies of what they unpack.
//!
//! `pull` and `fetch` use it only when `CI` is `1` or `true`. The `cache`
//! commands work on it anywhere, so a runner image can be warmed ahead of
//! time. The cache is **per user**, and there is deliberately no system-wide
//! scope for it: a cache several users write through would give anything that
//! poisons one entry a machine-wide reach.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use crate::store::{dir_size, entry_name, modified, older_than, touch, ImageStore, Lock};

/// Snapshot entries: `<cache>/snapshots/<entry>.git`.
const SNAPSHOTS: &str = "snapshots";
/// Image entries: `<cache>/images/<entry>/`, an OCI image layout.
const IMAGES: &str = "images";
/// Advisory locks, one file per entry name.
const LOCKS: &str = "locks";

/// Where the cache lives: `GITSCALE_CACHE_DIR`, else `$XDG_DATA_HOME/gitscale`,
/// else `~/.local/share/gitscale`. Every branch lands somewhere the invoking
/// user owns; with none of them set there is no cache at all.
pub fn resolve_dir() -> Option<PathBuf> {
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

/// The cache. Constructed once per command and shared across the threads
/// that operate on individual repositories.
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    /// The cache at its usual location, or `None` when this user has no data
    /// directory to keep one in.
    pub fn open() -> Option<Self> {
        Some(Self {
            root: resolve_dir()?,
        })
    }

    /// The cache a CI command uses automatically: none outside CI, and none
    /// with `--no-cache`.
    pub fn for_ci(no_cache: bool) -> Option<Self> {
        if no_cache || !crate::git::is_ci() {
            return None;
        }
        Self::open()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn snapshot_path(&self, url: &str) -> PathBuf {
        self.root.join(SNAPSHOTS).join(entry_name(url))
    }

    pub fn images(&self) -> ImageStore {
        ImageStore::new(self.root.join(IMAGES), self.root.join(LOCKS))
    }

    /// The snapshot entry's copy of the commit `revision` names, adding it if
    /// the entry does not have it yet.
    ///
    /// `Ok(None)` means this revision cannot be cached — an empty revision, a
    /// ref the remote does not advertise, or a bare SHA it refuses to serve —
    /// leaving the caller to fetch it the direct way.
    pub fn pin(&self, url: &str, revision: &str) -> Result<Option<Pinned>> {
        let Some(sha) = resolve_pin(url, revision)? else {
            return Ok(None);
        };
        let path = self.snapshot_path(url);
        let _lock = self.lock(&path)?;
        ensure_entry(&path, url)?;

        let reference = format!("refs/heads/pin/{}", sha);
        if !crate::git::ref_exists(&path, &format!("{}^{{commit}}", reference)) {
            // One commit, no history: everything this pin shares with a pin
            // already in the entry is already here.
            let fetched = git_in(&path, &["fetch", "--depth", "1", "origin", &sha], false)?;
            if !fetched.status.success() {
                return Ok(None);
            }
            git_in(&path, &["update-ref", &reference, "FETCH_HEAD"], true)?;
        }
        touch(&path.join(crate::store::LAST_USED));
        touch(&path.join(crate::store::PINS).join(&sha));
        Ok(Some(Pinned {
            entry: path,
            // `clone --branch` does not look outside `refs/heads`, which is
            // why pins live there and are named this way.
            reference: format!("pin/{}", sha),
            sha,
        }))
    }

    /// Every entry in the cache, of both kinds.
    pub fn entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        if let Ok(listing) = fs::read_dir(self.root.join(SNAPSHOTS)) {
            for found in listing.flatten() {
                let path = found.path();
                if crate::store::is_repository(&path) {
                    entries.push(Entry {
                        kind: Kind::Snapshot,
                        path,
                    });
                }
            }
        }
        for path in self.images().entries() {
            entries.push(Entry {
                kind: Kind::Image,
                path,
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries
    }

    /// What the cache costs on disk right now.
    pub fn usage(&self) -> Usage {
        let entries = self.entries();
        Usage {
            bytes: entries.iter().map(|e| dir_size(&e.path)).sum(),
            entries: entries.len(),
        }
    }

    /// What each entry costs, when it was last wanted, and which revisions it
    /// is holding.
    pub fn stats(&self) -> Vec<EntryStats> {
        self.entries()
            .into_iter()
            .map(|entry| EntryStats {
                bytes: dir_size(&entry.path),
                last_used: modified(&entry.path.join(crate::store::LAST_USED)),
                revisions: held_by(&entry),
                kind: entry.kind,
                path: entry.path,
            })
            .collect()
    }

    /// Drop what nothing has used for `keep`: whole entries, then the pins
    /// and images inside the ones that stay. Nothing borrows from an entry, so
    /// this prunes properly.
    pub fn compact(&self, keep: Duration) -> Result<Compacted> {
        let cutoff = SystemTime::now()
            .checked_sub(keep)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let mut report = Compacted::default();
        for entry in self.entries() {
            if entry.kind != Kind::Snapshot {
                continue;
            }
            let _lock = self.lock(&entry.path)?;
            let before = dir_size(&entry.path);
            if older_than(&entry.path.join(crate::store::LAST_USED), cutoff) {
                fs::remove_dir_all(&entry.path)
                    .with_context(|| format!("cannot remove {}", entry.path.display()))?;
                report.entries += 1;
                report.freed += before;
                continue;
            }
            report.pins += evict_pins(&entry.path, cutoff)?;
            git_in(&entry.path, &["gc", "--prune=now", "--quiet"], false)?;
            report.freed += before.saturating_sub(dir_size(&entry.path));
        }
        let images = self.images().prune(keep)?;
        report.entries += images.entries;
        report.pins += images.images;
        report.freed += images.freed;
        Ok(report)
    }

    /// Take an entry's lock. Held for as long as the returned guard lives, so
    /// N cold starts at once cost one download rather than N.
    fn lock(&self, entry: &Path) -> Result<Lock> {
        Lock::acquire(&self.root.join(LOCKS), entry)
    }
}

/// A commit taken out of a snapshot entry. CI clones it locally and copies the
/// objects, so nothing it produces depends on the entry surviving the job.
#[derive(Debug, Clone)]
pub struct Pinned {
    /// The bare snapshot entry holding the commit.
    pub entry: PathBuf,
    /// The ref inside it, as `git clone --branch` wants it: `pin/<sha>`.
    pub reference: String,
    /// The commit that ref points at.
    pub sha: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Snapshot,
    Image,
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
    /// Every revision the entry holds: a snapshot's pins, the commits an
    /// image entry has images for.
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
    /// Pins, and images, dropped from entries that stayed.
    pub pins: usize,
    pub freed: u64,
}

/// The revisions an entry holds, each with when it was last used.
fn held_by(entry: &Entry) -> Vec<Revision> {
    if entry.kind == Kind::Image {
        return crate::oci_layout::Layout::new(entry.path.clone())
            .held()
            .into_iter()
            .map(|held| Revision {
                last_used: ImageStore::used(&entry.path, &held.tag),
                name: held.tag,
            })
            .collect();
    }
    let Ok(listed) = git_in(
        &entry.path,
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads/pin/",
        ],
        false,
    ) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .map(|name| {
            let sha = name.strip_prefix("pin/").unwrap_or(name);
            Revision {
                // Each pin carries its own marker, so an entry in daily use
                // can still show which of its commits have gone cold.
                last_used: modified(&entry.path.join(crate::store::PINS).join(sha)),
                name: name.to_string(),
            }
        })
        .collect()
}

/// Drop pin refs nothing has taken since `cutoff`.
fn evict_pins(entry: &Path, cutoff: SystemTime) -> Result<usize> {
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
        let marker = entry.join(crate::store::PINS).join(sha);
        if !older_than(&marker, cutoff) {
            continue;
        }
        git_in(entry, &["update-ref", "-d", &reference], true)?;
        let _ = fs::remove_file(&marker);
        dropped += 1;
    }
    Ok(dropped)
}

/// Create the snapshot entry for `url` if it is not there, and keep its
/// `origin` pointing at the URL we would actually fetch from — which changes
/// under CI, where the same repository is reached over HTTPS with a job token.
fn ensure_entry(path: &Path, url: &str) -> Result<()> {
    if crate::store::is_repository(path) {
        if crate::git::origin_url(path).is_some() {
            crate::git::set_origin(path, url)?;
        } else {
            git_in(path, &["remote", "add", "origin", url], true)?;
        }
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
    git_in(&staging, &["remote", "add", "origin", url], true)?;
    fs::rename(&staging, path)
        .with_context(|| format!("cannot move the new entry into {}", path.display()))?;
    Ok(())
}

/// The commit `revision` names. A SHA needs no network at all; a branch or
/// tag costs one `ls-remote` — a ref advertisement, no objects.
fn resolve_pin(url: &str, revision: &str) -> Result<Option<String>> {
    if revision.is_empty() {
        return Ok(None);
    }
    if crate::git::is_full_sha(revision) {
        return Ok(Some(revision.to_lowercase()));
    }
    Ok(crate::git::ls_remote_revision(url, revision, false)?.map(|(sha, _)| sha))
}

/// Run git inside a bare cache entry.
fn git_in(entry: &Path, args: &[&str], check: bool) -> Result<std::process::Output> {
    let dir = entry.to_string_lossy().to_string();
    let mut full: Vec<&str> = vec!["-C", &dir];
    full.extend_from_slice(args);
    crate::git::run_git(&full, None, check)
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

/// Parse a period in humantime's syntax, as in `1month`, `30d` or `2 weeks`.
///
/// A bare `m` or `M` is refused: humantime reads them as minutes and months,
/// and taking one for the other here would evict nearly everything.
pub fn parse_period(text: &str) -> Result<Duration> {
    static BARE_M: OnceLock<regex::Regex> = OnceLock::new();
    let bare_m = BARE_M.get_or_init(|| regex::Regex::new(r"\d\s*[mM](?:$|[\s\d])").unwrap());
    if bare_m.is_match(text.trim()) {
        bail!(
            "'{}' is ambiguous: write 'min' for minutes or 'months' for months",
            text
        );
    }
    humantime::parse_duration(text.trim())
        .map_err(|e| anyhow::anyhow!("invalid period '{}': {}", text, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_parse_the_way_the_flag_spells_them() {
        // humantime's month and year: 30.44 and 365.25 days.
        assert_eq!(
            parse_period("1month").unwrap(),
            Duration::from_secs(2_630_016)
        );
        assert_eq!(
            parse_period("12months").unwrap(),
            Duration::from_secs(12 * 2_630_016)
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
        assert_eq!(
            parse_period("1d 12h").unwrap(),
            Duration::from_secs(36 * 3_600)
        );
        assert!(parse_period("soon").is_err());
        assert!(parse_period("3 fortnights").is_err());
        // A count needs a unit.
        assert!(parse_period("7").is_err());
    }

    #[test]
    fn small_units_are_not_read_as_large_ones() {
        assert_eq!(parse_period("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_period("10min").unwrap(), Duration::from_secs(600));
        assert_eq!(parse_period("10ms").unwrap(), Duration::from_millis(10));
    }

    #[test]
    fn a_bare_m_is_refused_as_ambiguous() {
        for text in ["6m", "6M", "6 m", "1h30m", "1m 2d"] {
            let err = parse_period(text).unwrap_err().to_string();
            assert!(err.contains("ambiguous"), "{}: {}", text, err);
        }
        assert!(parse_period("6min").is_ok());
        assert!(parse_period("6months").is_ok());
    }

    #[test]
    fn an_overflowing_period_is_an_error_not_a_panic() {
        assert!(parse_period("99999999999999999999y").is_err());
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

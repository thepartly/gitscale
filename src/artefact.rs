//! Artefact mode: prebuilt files published to an OCI registry by the source
//! repository's pipeline, one image per commit, and unpacked read-only into
//! the workspace instead of a git checkout.
//!
//! Two halves. The producer runs `gitscale artefact publish`: the globs in its
//! own `[artefact]` table pick the files, each group becomes one reproducible
//! gzip tar layer, and the image is tagged with the full commit it was built
//! from. The consumer declares an entry with `mode = "artefact"`: its revision
//! is resolved to a commit with `git ls-remote`, exactly as a git entry's
//! would be, and that commit's image is what gets installed. Git decides which
//! commit a revision means; the registry only stores one image per commit.

use anyhow::{bail, Context, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::cache::Cache;
use crate::config::{
    find_config, load_config, ArtefactSpec, GitScaleConfig, RepoEntry, CONFIG_FILENAME,
};
use crate::git::{is_full_sha, short_sha};
use crate::registry::{
    image_for, Access, Client, HashingWriter, Image, CONFIG_MEDIA_TYPE, LAYER_MEDIA_TYPE,
    MANIFEST_MEDIA_TYPE,
};

const TITLE: &str = "org.opencontainers.image.title";

/// The layer `publish` adds to every image, first, carrying the repository's
/// own `.gitscale.toml` — so a consumer can read an artefact's dependencies
/// by downloading a few hundred bytes, before deciding anything else. A user
/// group may not take the name.
pub const CONFIG_LAYER: &str = "gitscale";
const REVISION: &str = "org.opencontainers.image.revision";
const SOURCE: &str = "org.opencontainers.image.source";

// ---------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------

/// Whether `pattern` is a glob `[artefact]` accepts: relative to `root`,
/// staying below it, and one `globset` compiles.
pub fn check_pattern(pattern: &str) -> Result<()> {
    if pattern.is_empty() {
        bail!("an empty pattern matches nothing");
    }
    if pattern.starts_with('/') {
        bail!("\"{}\" must be relative to the artefact root", pattern);
    }
    if pattern.split('/').any(|part| part == ".." || part == ".") {
        bail!(
            "\"{}\" must not contain '.' or '..' path components",
            pattern
        );
    }
    glob(pattern).map(|_| ())
}

fn glob(pattern: &str) -> Result<globset::Glob> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .with_context(|| format!("invalid pattern \"{}\"", pattern))
}

fn glob_set(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(glob(pattern)?);
    }
    Ok(builder.build()?)
}

/// `set` matches `rel` itself or a directory it is in — naming a directory
/// includes everything below it.
fn matches_path(set: &GlobSet, rel: &str) -> bool {
    if set.is_match(rel) {
        return true;
    }
    rel.match_indices('/')
        .any(|(at, _)| set.is_match(&rel[..at]))
}

struct Group {
    include: GlobSet,
    exclude: GlobSet,
}

impl Group {
    fn matches(&self, rel: &str) -> bool {
        matches_path(&self.include, rel) && !matches_path(&self.exclude, rel)
    }
}

// ---------------------------------------------------------------------------
// Packing (the producer)
// ---------------------------------------------------------------------------

/// One group, packed.
pub struct PackedLayer {
    pub name: String,
    pub files: Vec<String>,
    /// The gzip tar, in the publish's work directory.
    pub path: PathBuf,
    pub digest: String,
    pub size: u64,
    /// The digest of the uncompressed tar, as an image config records it.
    pub diff_id: String,
}

/// Every file and symlink under `root`, as `/`-separated paths relative to
/// it, sorted. `.git` is never part of an artefact. Directories are walked,
/// not recorded; symlinks are recorded, not followed, and one whose target
/// leaves `root` is refused.
pub fn collect_files(root: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk(root: &Path, dir: &Path, files: &mut Vec<String>) -> Result<()> {
    let listing = fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))?;
    for found in listing {
        let found = found?;
        if found.file_name() == ".git" {
            continue;
        }
        let path = found.path();
        let rel = path
            .strip_prefix(root)
            .ok()
            .and_then(|r| r.to_str())
            .ok_or_else(|| anyhow::anyhow!("{} is not a UTF-8 path", path.display()))?
            .to_string();
        let kind = found.file_type()?;
        if kind.is_symlink() {
            let target = fs::read_link(&path)?;
            if !link_stays_inside(Path::new(&rel), &target) {
                bail!(
                    "{} is a symlink to {}, outside the artefact root",
                    rel,
                    target.display()
                );
            }
            files.push(rel);
        } else if kind.is_dir() {
            walk(root, &path, files)?;
        } else if kind.is_file() {
            files.push(rel);
        } else {
            bail!("{} is neither a file, a directory nor a symlink", rel);
        }
    }
    Ok(())
}

/// Whether a symlink at `link` (relative to the root) pointing at `target`
/// resolves inside the root, judged on the path alone.
fn link_stays_inside(link: &Path, target: &Path) -> bool {
    if target.is_absolute() {
        return false;
    }
    let mut depth: i32 = 0;
    let parent = link.parent().unwrap_or(Path::new(""));
    for component in parent.components().chain(target.components()) {
        match component {
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            Component::CurDir => {}
            _ => return false,
        }
    }
    true
}

/// Which files go in which group: the first group that matches a file takes
/// it. A group that matches nothing is an error — a broken build or a typo
/// must not publish an artefact with a hole in it.
pub fn assign(spec: &ArtefactSpec, files: &[String]) -> Result<Vec<(String, Vec<String>)>> {
    let groups: Vec<Group> = spec
        .layers
        .iter()
        .map(|layer| {
            Ok(Group {
                include: glob_set(&layer.include)?,
                exclude: glob_set(&layer.exclude)?,
            })
        })
        .collect::<Result<_>>()?;
    let mut assigned: Vec<Vec<String>> = vec![Vec::new(); groups.len()];
    for file in files {
        if let Some(i) = groups.iter().position(|g| g.matches(file)) {
            assigned[i].push(file.clone());
        }
    }
    for (layer, files) in spec.layers.iter().zip(&assigned) {
        if files.is_empty() {
            bail!(
                "layer \"{}\" matches no files under {}",
                layer.name,
                spec.root
            );
        }
    }
    Ok(spec
        .layers
        .iter()
        .map(|l| l.name.clone())
        .zip(assigned)
        .collect())
}

/// Pack the files `spec` selects under `base` into one layer per group,
/// written to `work`.
pub fn pack(spec: &ArtefactSpec, base: &Path, work: &Path) -> Result<Vec<PackedLayer>> {
    let root = base.join(&spec.root);
    if !root.is_dir() {
        bail!(
            "the artefact root {} does not exist — has the build run?",
            root.display()
        );
    }
    // The top-level `.gitscale.toml` of every image is the repository's own,
    // carried by the config layer; one under the artefact root would clash
    // with it, and is not shipped.
    let files: Vec<String> = collect_files(&root)?
        .into_iter()
        .filter(|f| f != CONFIG_FILENAME)
        .collect();
    let groups = assign(spec, &files)?;
    fs::create_dir_all(work)?;
    groups
        .into_iter()
        .enumerate()
        .map(|(i, (name, files))| {
            let path = work.join(format!("layer-{}.tar.gz", i));
            let (digest, size, diff_id) = write_layer(&root, &files, &path)
                .with_context(|| format!("cannot pack layer \"{}\"", name))?;
            Ok(PackedLayer {
                name,
                files,
                path,
                digest,
                size,
                diff_id,
            })
        })
        .collect()
}

/// The config layer: `base`'s `.gitscale.toml`, alone, at the top of the
/// image.
pub fn pack_config(base: &Path, work: &Path) -> Result<PackedLayer> {
    fs::create_dir_all(work)?;
    let files = vec![CONFIG_FILENAME.to_string()];
    let path = work.join("layer-config.tar.gz");
    let (digest, size, diff_id) = write_layer(base, &files, &path)
        .with_context(|| format!("cannot pack layer \"{}\"", CONFIG_LAYER))?;
    Ok(PackedLayer {
        name: CONFIG_LAYER.to_string(),
        files,
        path,
        digest,
        size,
        diff_id,
    })
}

/// Write `files` as a reproducible gzip tar: entries in the order given,
/// ownership and timestamps fixed, modes reduced to 0644 or 0755, and a gzip
/// header with no time or name. The same files always give the same bytes.
fn write_layer(root: &Path, files: &[String], out: &Path) -> Result<(String, u64, String)> {
    let file = fs::File::create(out).with_context(|| format!("cannot create {}", out.display()))?;
    let compressed = HashingWriter::new(file);
    let gzip = flate2::GzBuilder::new()
        .mtime(0)
        .write(compressed, flate2::Compression::default());
    let tarred = HashingWriter::new(gzip);
    let mut builder = tar::Builder::new(tarred);
    builder.mode(tar::HeaderMode::Deterministic);
    builder.follow_symlinks(false);
    for rel in files {
        builder
            .append_path_with_name(root.join(rel), rel)
            .with_context(|| format!("cannot add {}", rel))?;
    }
    let tarred = builder.into_inner()?;
    let (gzip, diff_id, _) = tarred.finish();
    let compressed = gzip.finish()?;
    let (file, digest, size) = compressed.finish();
    file.sync_all()?;
    Ok((digest, size, diff_id))
}

/// The image config: enough to make the artefact a valid image any registry
/// and tool accepts, and nothing that changes between two builds of the same
/// files.
pub fn image_config(layers: &[PackedLayer]) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "architecture": "unknown",
        "os": "unknown",
        "config": {},
        "rootfs": {
            "type": "layers",
            "diff_ids": layers.iter().map(|l| l.diff_id.clone()).collect::<Vec<_>>(),
        },
    }))
    .expect("a JSON value always serializes")
}

/// The manifest. No creation time, so its digest is reproducible too.
pub fn manifest(
    config_digest: &str,
    config_size: u64,
    layers: &[PackedLayer],
    commit: &str,
    source: &str,
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST_MEDIA_TYPE,
        "config": {
            "mediaType": CONFIG_MEDIA_TYPE,
            "digest": config_digest,
            "size": config_size,
        },
        "layers": layers.iter().map(|l| serde_json::json!({
            "mediaType": LAYER_MEDIA_TYPE,
            "digest": l.digest,
            "size": l.size,
            "annotations": { TITLE: l.name },
        })).collect::<Vec<_>>(),
        "annotations": {
            REVISION: commit,
            SOURCE: source,
        },
    }))
    .expect("a JSON value always serializes")
}

// ---------------------------------------------------------------------------
// Reading an image (the consumer)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Manifest {
    #[serde(default)]
    layers: Vec<LayerDescriptor>,
}

#[derive(Deserialize)]
struct LayerDescriptor {
    #[serde(rename = "mediaType", default)]
    media_type: String,
    digest: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}

impl LayerDescriptor {
    /// Whether the layer is gzipped; an error for a kind gitscale cannot
    /// unpack.
    fn gzip(&self) -> Result<bool> {
        match self.media_type.as_str() {
            "application/vnd.oci.image.layer.v1.tar+gzip"
            | "application/vnd.docker.image.rootfs.diff.tar.gzip" => Ok(true),
            "application/vnd.oci.image.layer.v1.tar" => Ok(false),
            other => bail!(
                "layer {} has a media type gitscale cannot unpack: {}",
                self.digest,
                other
            ),
        }
    }
}

fn parse_manifest(bytes: &[u8]) -> Result<Manifest> {
    let manifest: Manifest =
        serde_json::from_slice(bytes).context("the image manifest is not valid JSON")?;
    for layer in &manifest.layers {
        if !crate::registry::valid_digest(&layer.digest) {
            bail!(
                "the image manifest names an invalid layer digest: {}",
                layer.digest
            );
        }
    }
    Ok(manifest)
}

// ---------------------------------------------------------------------------
// Markers and state
// ---------------------------------------------------------------------------

/// What a marker file records about one image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    /// The entry's revision when this was written: a config that now says
    /// something else makes the checkout a ref mismatch.
    pub revision: String,
    pub commit: String,
    /// `None` in a fetch's marker when the commit has no image yet.
    pub digest: Option<String>,
    pub image: String,
}

fn read_marker(path: &Path) -> Option<Marker> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn write_marker(path: &Path, marker: &Marker) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let mut text = serde_json::to_vec_pretty(marker)?;
    text.push(b'\n');
    fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
}

/// Where gitscale records what it installed into one artefact checkout, and
/// what the last fetch saw for it.
///
/// Never inside the checkout: every file there belongs to the artefact, dot
/// files included, and a name gitscale reserved there would be one an
/// artefact could not ship. The records live in the workspace repository's
/// git directory — per worktree, since each has checkouts of its own — or,
/// for a workspace that is not the top of a repository, in `.gitscale/`
/// beside its config.
pub struct Markers {
    installed: PathBuf,
    remote: PathBuf,
}

impl Markers {
    pub fn new(config_root: &Path, directory: &str) -> Markers {
        let dir = if crate::git::is_repo_root(config_root) {
            crate::git::git_path(config_root, "gitscale/artefacts")
        } else {
            None
        }
        .unwrap_or_else(|| config_root.join(".gitscale/artefacts"));
        // Readable, and unique: two workspaces in one repository may declare
        // the same directory.
        let absolute = fs::canonicalize(config_root)
            .unwrap_or_else(|_| config_root.to_path_buf())
            .join(directory);
        let digest = crate::registry::sha256_digest(absolute.to_string_lossy().as_bytes());
        let slug: String = directory
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let key = format!("{}-{}", slug.trim_matches(['-', '.']), &digest[7..19]);
        Markers {
            installed: dir.join(format!("{}.installed.json", key)),
            remote: dir.join(format!("{}.remote.json", key)),
        }
    }
}

/// Whether an artefact has been installed into `dest`: the record says so,
/// and the files are there. A checkout somebody deleted is not installed,
/// whatever the record remembers.
pub fn is_downloaded(markers: &Markers, dest: &Path) -> bool {
    !is_empty_dir(dest) && read_marker(&markers.installed).is_some()
}

/// The commit installed into the artefact checkout at `directory`, if any.
pub fn installed_commit(config_root: &Path, directory: &str) -> Option<String> {
    let dest = config_root.join(directory);
    read_marker(&Markers::new(config_root, directory).installed)
        .filter(|_| !is_empty_dir(&dest))
        .map(|m| m.commit)
}

/// Drop what gitscale recorded about the artefact checkout at `directory`,
/// once the checkout itself is gone.
pub fn forget_install(config_root: &Path, directory: &str) {
    let markers = Markers::new(config_root, directory);
    let _ = fs::remove_file(&markers.installed);
    let _ = fs::remove_file(&markers.remote);
}

/// Whether `dest` is missing or holds nothing at all.
pub fn is_empty_dir(dest: &Path) -> bool {
    fs::read_dir(dest).map_or(true, |mut entries| entries.next().is_none())
}

/// An artefact checkout as `status` reports it.
#[derive(Debug, Clone, Default)]
pub struct State {
    pub installed: Option<Marker>,
    pub remote: Option<Marker>,
    /// `behind`, `missing`, `changed`, `ref-mismatch` — see [`state`].
    pub flags: Vec<String>,
}

/// What `dest` holds for `entry`, compared with what the last fetch saw.
///
/// * `ref-mismatch` — installed for another revision than the config names
///   now, or not at the commit a SHA revision pins;
/// * `behind` — the revision has moved to another commit since;
/// * `missing` — that commit has no image (yet);
/// * `changed` — the installed commit's image was re-published with
///   different content.
pub fn state(entry: &RepoEntry, config_root: &Path) -> State {
    state_at(
        entry,
        &config_root.join(&entry.directory),
        &Markers::new(config_root, &entry.directory),
    )
}

fn state_at(entry: &RepoEntry, dest: &Path, markers: &Markers) -> State {
    let installed = read_marker(&markers.installed).filter(|_| !is_empty_dir(dest));
    let remote = read_marker(&markers.remote).filter(|r| r.revision == entry.revision);
    let flags = flags(entry, installed.as_ref(), remote.as_ref());
    State {
        installed,
        remote,
        flags,
    }
}

/// How what is installed compares with the configured revision and with what
/// the remote has — see [`state`].
fn flags(entry: &RepoEntry, installed: Option<&Marker>, remote: Option<&Marker>) -> Vec<String> {
    let mut flags = Vec::new();
    let Some(installed) = installed else {
        return flags;
    };
    let pinned =
        is_full_sha(&entry.revision) && !installed.commit.eq_ignore_ascii_case(&entry.revision);
    if installed.revision != entry.revision || pinned {
        flags.push("ref-mismatch".to_string());
    }
    if let Some(remote) = remote {
        if remote.commit != installed.commit {
            flags.push("behind".to_string());
            if remote.digest.is_none() {
                flags.push("missing".to_string());
            }
        } else if remote.digest.is_some() && remote.digest != installed.digest {
            flags.push("changed".to_string());
        }
    }
    flags
}

// ---------------------------------------------------------------------------
// Unpacking
// ---------------------------------------------------------------------------

/// Unpack one layer into `dest`. Only files, directories and symlinks that
/// stay inside `dest` are accepted; anything else fails the whole install.
fn extract_layer(blob: &Path, dest: &Path, gzip: bool) -> Result<()> {
    let file = fs::File::open(blob)?;
    let reader: Box<dyn std::io::Read> = if gzip {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_permissions(true);
    // Files land with the time they were unpacked: the archive's fixed one
    // would make every build tool think them older than anything it built.
    archive.set_preserve_mtime(false);
    archive.set_overwrite(true);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let shown = path.display().to_string();
        let safe = !path.as_os_str().is_empty()
            && path
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
        if !safe {
            bail!("artefact archive contains unsafe path: {}", shown);
        }
        match entry.header().entry_type() {
            tar::EntryType::Regular | tar::EntryType::Directory => {}
            tar::EntryType::Symlink => {
                let target = entry
                    .link_name()?
                    .ok_or_else(|| anyhow::anyhow!("symlink {} has no target", shown))?
                    .into_owned();
                if !link_stays_inside(&path, &target) {
                    bail!(
                        "artefact archive contains a symlink leaving the checkout: {} -> {}",
                        shown,
                        target.display()
                    );
                }
            }
            other => bail!(
                "artefact archive contains an unsupported entry {} ({:?})",
                shown,
                other
            ),
        }
        if !entry.unpack_in(dest)? {
            bail!("artefact archive contains unsafe path: {}", shown);
        }
    }
    Ok(())
}

/// Clear every write bit (`writable = false`) or restore the owner's on each
/// file under `dest`, at any depth and whatever it is called — an artefact's
/// `.git` or dot files are its own like everything else. Symlinks are left
/// alone.
fn set_write_bits(dest: &Path, writable: bool) -> Result<()> {
    let Ok(listing) = fs::read_dir(dest) else {
        return Ok(());
    };
    for found in listing {
        let found = found?;
        let kind = found.file_type()?;
        let path = found.path();
        if kind.is_dir() {
            set_write_bits(&path, writable)?;
        } else if kind.is_file() {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path)?.permissions();
            let mode = perms.mode();
            perms.set_mode(if writable {
                mode | 0o200
            } else {
                mode & !0o222
            });
            fs::set_permissions(&path, perms)?;
        }
    }
    Ok(())
}

fn apply_readonly(dest: &Path) -> Result<()> {
    set_write_bits(dest, false)
}

fn restore_writable(dest: &Path) -> Result<()> {
    set_write_bits(dest, true)
}

/// Delete everything in `dest`.
fn clean_files(dest: &Path) -> Result<()> {
    for found in fs::read_dir(dest)? {
        let found = found?;
        let path = found.path();
        if found.file_type()?.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        }
        .with_context(|| format!("cannot remove {}", path.display()))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Consumer operations
// ---------------------------------------------------------------------------

/// What one command needs to work with artefacts: where images live, a
/// registry client, and the cache, if it is on.
pub struct Artefacts {
    config_root: PathBuf,
    registries: BTreeMap<String, String>,
    client: Client,
    cache: Option<Cache>,
}

/// What [`Artefacts::describe`] found.
pub struct Description {
    pub image: Image,
    /// The commit the entry's revision names on the remote now.
    pub commit: String,
    /// The image digest of that commit, `None` when it is not published.
    pub digest: Option<String>,
    pub layers: Vec<Layer>,
    pub installed: Option<Marker>,
    /// `behind`, `missing`, `changed`, `ref-mismatch`, against the remote now.
    pub flags: Vec<String>,
}

/// One layer of a published image.
pub struct Layer {
    /// The group it was packed from, when the publisher recorded one.
    pub title: String,
    pub size: u64,
    pub digest: String,
}

/// What `pull` did.
pub enum Pulled {
    Updated(String),
    Current(String),
}

/// The blobs an install unpacks, and whatever has to stay alive while it
/// does: the cache entry's lock, or the temporary directory the layers were
/// downloaded to.
struct Obtained {
    layers: Vec<(PathBuf, bool)>,
    _lock: Option<crate::cache::Lock>,
    temp: Option<PathBuf>,
}

impl Drop for Obtained {
    fn drop(&mut self) {
        if let Some(temp) = &self.temp {
            let _ = fs::remove_dir_all(temp);
        }
    }
}

impl Artefacts {
    pub fn new(config: &GitScaleConfig, config_root: &Path, cache: Option<Cache>) -> Self {
        Artefacts {
            config_root: config_root.to_path_buf(),
            registries: config.registries.clone(),
            client: Client::new(),
            cache,
        }
    }

    /// The image `entry` is published as, and the commit its revision names
    /// right now.
    fn locate(&self, entry: &RepoEntry) -> Result<(Image, String)> {
        let image = image_for(&entry.repo_url, &self.registries)?;
        let commit =
            crate::git::resolve_remote_commit(&crate::git::remote_url(entry), &entry.revision)?;
        Ok((image, commit))
    }

    fn markers(&self, entry: &RepoEntry) -> Markers {
        Markers::new(&self.config_root, &entry.directory)
    }

    /// The digest of `commit`'s image, or the error that says it has none.
    fn require(&self, entry: &RepoEntry, image: &Image, commit: &str) -> Result<String> {
        self.client
            .manifest_digest(image, commit)?
            .ok_or_else(|| missing(entry, image, commit))
    }

    /// Install the image of the commit `entry`'s revision names into `dest`.
    pub fn clone(&self, entry: &RepoEntry, dest: &Path) -> Result<String> {
        let (image, commit) = self.locate(entry)?;
        let digest = self.require(entry, &image, &commit)?;
        self.install(entry, dest, &image, &commit, &digest)?;
        Ok(commit)
    }

    /// Bring `dest` to the commit `entry`'s revision names now. Nothing to do,
    /// and nothing asked of the registry, when it is already there — unless a
    /// fetch saw that commit's image re-published since.
    pub fn pull(&self, entry: &RepoEntry, dest: &Path) -> Result<Pulled> {
        let (image, commit) = self.locate(entry)?;
        let state = state_at(entry, dest, &self.markers(entry));
        if let Some(installed) = &state.installed {
            let republished = state.flags.iter().any(|f| f == "changed");
            if installed.commit == commit && installed.revision == entry.revision && !republished {
                return Ok(Pulled::Current(commit));
            }
        }
        let digest = self.require(entry, &image, &commit)?;
        self.install(entry, dest, &image, &commit, &digest)?;
        Ok(Pulled::Updated(commit))
    }

    /// Record what the remote has for `entry` now, without downloading it:
    /// the commit its revision names, and whether that commit has an image.
    pub fn fetch(&self, entry: &RepoEntry) -> Result<String> {
        let (image, commit) = self.locate(entry)?;
        let digest = self.client.manifest_digest(&image, &commit)?;
        write_marker(
            &self.markers(entry).remote,
            &Marker {
                revision: entry.revision.clone(),
                commit: commit.clone(),
                digest: digest.clone(),
                image: image.reference(),
            },
        )?;
        if digest.is_none() {
            return Err(missing(entry, &image, &commit));
        }
        Ok(commit)
    }

    /// Everything there is to know about `entry` right now, asked of the
    /// remote and the registry, changing nothing: the image, the commit the
    /// revision names, whether that commit is published and with what layers,
    /// what is installed, and how the two compare.
    pub fn describe(&self, entry: &RepoEntry) -> Result<Description> {
        let (image, commit) = self.locate(entry)?;
        let digest = self.client.manifest_digest(&image, &commit)?;
        let layers = match &digest {
            Some(digest) => parse_manifest(&self.client.manifest(&image, digest)?)?
                .layers
                .into_iter()
                .map(|l| Layer {
                    title: l.annotations.get(TITLE).cloned().unwrap_or_default(),
                    size: l.size,
                    digest: l.digest,
                })
                .collect(),
            None => Vec::new(),
        };
        let markers = self.markers(entry);
        let dest = self.config_root.join(&entry.directory);
        let installed = read_marker(&markers.installed).filter(|_| !is_empty_dir(&dest));
        let now = Marker {
            revision: entry.revision.clone(),
            commit: commit.clone(),
            digest: digest.clone(),
            image: image.reference(),
        };
        let flags = flags(entry, installed.as_ref(), Some(&now));
        Ok(Description {
            image,
            commit,
            digest,
            layers,
            installed,
            flags,
        })
    }

    /// The image `entry` is published as, and every tag the registry holds
    /// for it — one per published commit.
    pub fn published(&self, entry: &RepoEntry) -> Result<(Image, Vec<String>)> {
        let image = image_for(&entry.repo_url, &self.registries)?;
        let tags = self.client.tags(&image)?;
        Ok((image, tags))
    }

    /// What is installed for `entry`, if anything.
    pub fn installed(&self, entry: &RepoEntry) -> Option<Marker> {
        let dest = self.config_root.join(&entry.directory);
        read_marker(&self.markers(entry).installed).filter(|_| !is_empty_dir(&dest))
    }

    /// Download the image `entry`'s revision names into the cache, touching
    /// no checkout. `Ok(None)` when the cache is off.
    pub fn warm(&self, entry: &RepoEntry) -> Result<Option<String>> {
        if self.cache.is_none() {
            return Ok(None);
        }
        let (image, commit) = self.locate(entry)?;
        let digest = self.require(entry, &image, &commit)?;
        self.obtain(entry, &image, &commit, &digest)?;
        Ok(Some(commit))
    }

    /// Re-check every blob `entry`'s cache entry holds, deleting the damaged
    /// ones. `None` when there is no entry.
    pub fn repair(&self, entry: &RepoEntry) -> Result<Option<usize>> {
        let Some(cache) = &self.cache else {
            return Ok(None);
        };
        let path = cache.artefact_path(&crate::git::remote_url(entry));
        if !crate::oci_layout::Layout::exists(&path) {
            return Ok(None);
        }
        let _lock = cache.lock(&path)?;
        crate::oci_layout::Layout::new(path).verify().map(Some)
    }

    /// The `.gitscale.toml` the image of `commit` carries in its config
    /// layer, for resolution: from the cache when it holds it, else — when
    /// `online` — downloaded into it. `None` for an image without one.
    pub fn config_layer(&self, url: &str, commit: &str, online: bool) -> Result<Option<String>> {
        let not_here = || {
            crate::resolution::unavailable(format!(
                "the artefact of {} at {} is not on this machine",
                url,
                short_sha(commit)
            ))
        };
        let image = image_for(url, &self.registries)?;
        let entry = self
            .cache
            .as_ref()
            .map(|cache| (cache, cache.artefact_path(&crate::ci::remote_url(url))));
        let _lock = match &entry {
            Some((cache, path)) => Some(cache.lock(path)?),
            None => None,
        };
        let layout = entry
            .as_ref()
            .map(|(_, path)| crate::oci_layout::Layout::new(path.clone()));

        let held = layout.as_ref().and_then(|layout| {
            let found = layout.held().into_iter().find(|h| h.commit == commit)?;
            let blob = layout.verified_blob(&found.digest)?;
            fs::read(blob).ok()
        });
        let manifest_bytes = match held {
            Some(bytes) => bytes,
            None if !online => return Err(not_here()),
            None => {
                let digest = self
                    .client
                    .manifest_digest(&image, commit)?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "no artefact for {}:{}; its pipeline may not have published yet",
                            image.reference(),
                            short_sha(commit)
                        )
                    })?;
                let bytes = self.client.manifest(&image, &digest)?;
                if let Some(layout) = &layout {
                    layout.ensure()?;
                    let target = layout.blob_path(&digest)?;
                    let partial = target.with_extension("partial");
                    fs::write(&partial, &bytes)?;
                    fs::rename(&partial, &target)?;
                    layout.record(commit, &digest, bytes.len() as u64)?;
                }
                bytes
            }
        };
        let manifest = parse_manifest(&manifest_bytes)?;
        let Some(layer) = manifest
            .layers
            .iter()
            .find(|l| l.annotations.get(TITLE).map(String::as_str) == Some(CONFIG_LAYER))
        else {
            return Ok(None);
        };
        let gzip = layer.gzip()?;
        let (blob, _temp) = match layout.as_ref().and_then(|l| l.verified_blob(&layer.digest)) {
            Some(found) => (found, None),
            None if !online => return Err(not_here()),
            None => match &layout {
                Some(layout) => {
                    let target = layout.blob_path(&layer.digest)?;
                    self.client.download_blob(&image, &layer.digest, &target)?;
                    (target, None)
                }
                None => {
                    let temp = WorkDir(unique_temp("config-layer"));
                    fs::create_dir_all(&temp.0)?;
                    let target = temp.0.join("layer");
                    self.client.download_blob(&image, &layer.digest, &target)?;
                    (target, Some(temp))
                }
            },
        };
        read_from_layer(&blob, gzip, CONFIG_FILENAME)
    }

    fn install(
        &self,
        entry: &RepoEntry,
        dest: &Path,
        image: &Image,
        commit: &str,
        digest: &str,
    ) -> Result<()> {
        // Everything is downloaded and checked before the old files go, so a
        // registry that fails part way leaves the installed version alone.
        let obtained = self.obtain(entry, image, commit, digest)?;
        fs::create_dir_all(dest).with_context(|| format!("cannot create {}", dest.display()))?;
        let markers = self.markers(entry);
        // Gone until the new files are all in, so a failure part way is not
        // taken for an installed artefact next time.
        let _ = fs::remove_file(&markers.installed);
        restore_writable(dest)?;
        clean_files(dest)?;
        let unpacked = obtained
            .layers
            .iter()
            .try_for_each(|(blob, gzip)| extract_layer(blob, dest, *gzip));
        if let Err(e) = unpacked {
            // Take back what was unpacked, so the next attempt starts over.
            let _ = restore_writable(dest).and_then(|()| clean_files(dest));
            return Err(e);
        }
        apply_readonly(dest)?;
        let marker = Marker {
            revision: entry.revision.clone(),
            commit: commit.to_string(),
            digest: Some(digest.to_string()),
            image: image.reference(),
        };
        write_marker(&markers.remote, &marker)?;
        // Last: it is what says the artefact is here.
        write_marker(&markers.installed, &marker)
    }

    /// The layers of the image `digest`, on local disk and checked: from the
    /// cache, downloading into it whatever it does not hold, or — with the
    /// cache off — downloaded to a temporary directory beside the checkout.
    fn obtain(
        &self,
        entry: &RepoEntry,
        image: &Image,
        commit: &str,
        digest: &str,
    ) -> Result<Obtained> {
        let Some(cache) = &self.cache else {
            let temp = unique_temp("download");
            fs::create_dir_all(&temp)
                .with_context(|| format!("cannot create {}", temp.display()))?;
            let mut obtained = Obtained {
                layers: Vec::new(),
                _lock: None,
                temp: Some(temp.clone()),
            };
            let manifest = parse_manifest(&self.client.manifest(image, digest)?)?;
            for layer in &manifest.layers {
                let gzip = layer.gzip()?;
                let path = temp.join(layer.digest.trim_start_matches("sha256:"));
                self.client.download_blob(image, &layer.digest, &path)?;
                obtained.layers.push((path, gzip));
            }
            return Ok(obtained);
        };

        let path = cache.artefact_path(&crate::git::remote_url(entry));
        // Held until the install has unpacked everything: a compaction must
        // not delete a blob between the download and the extraction, and N
        // cold jobs at once download each blob once.
        let lock = cache.lock(&path)?;
        let layout = crate::oci_layout::Layout::new(path.clone());
        layout.ensure()?;
        let manifest_path = match layout.verified_blob(digest) {
            Some(found) => found,
            None => {
                let bytes = self.client.manifest(image, digest)?;
                let target = layout.blob_path(digest)?;
                let partial = target.with_extension("partial");
                fs::write(&partial, &bytes)?;
                fs::rename(&partial, &target)?;
                target
            }
        };
        let bytes = fs::read(&manifest_path)?;
        let manifest = parse_manifest(&bytes)?;
        let mut layers = Vec::new();
        for layer in &manifest.layers {
            let gzip = layer.gzip()?;
            let blob = match layout.verified_blob(&layer.digest) {
                Some(found) => found,
                None => {
                    let target = layout.blob_path(&layer.digest)?;
                    self.client.download_blob(image, &layer.digest, &target)?;
                    target
                }
            };
            layers.push((blob, gzip));
        }
        layout.record(commit, digest, bytes.len() as u64)?;
        cache.touch_artefact(&path, commit);
        Ok(Obtained {
            layers,
            _lock: Some(lock),
            temp: None,
        })
    }
}

/// The file at `name` in the tar layer `blob`, if the layer has one.
fn read_from_layer(blob: &Path, gzip: bool, name: &str) -> Result<Option<String>> {
    use std::io::Read;
    let file = fs::File::open(blob).with_context(|| format!("cannot open {}", blob.display()))?;
    let reader: Box<dyn Read> = if gzip {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let path = path.strip_prefix(".").unwrap_or(&path);
        if path == Path::new(name) {
            let mut text = String::new();
            entry.read_to_string(&mut text)?;
            return Ok(Some(text));
        }
    }
    Ok(None)
}

/// A directory under the system temp dir that no other operation — in this
/// process or another — will be handed.
fn unique_temp(purpose: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "gitscale-{}-{}-{}",
        purpose,
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// The error for a commit with no image.
fn missing(entry: &RepoEntry, image: &Image, commit: &str) -> anyhow::Error {
    let revision = if entry.revision.is_empty() {
        "default branch".to_string()
    } else {
        entry.revision.clone()
    };
    anyhow::anyhow!(
        "no artefact for {}:{} ({}); its pipeline may not have published yet",
        image.reference(),
        short_sha(commit),
        revision
    )
}

// ---------------------------------------------------------------------------
// Publishing
// ---------------------------------------------------------------------------

/// A work directory removed when the publish is done, however it ends.
struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `gitscale artefact publish`: pack what the `[artefact]` table of the
/// repository at `root` selects and push it as the image for one commit.
pub fn publish(
    root: Option<&Path>,
    commit: Option<&str>,
    force: bool,
    dry_run: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let config_path = find_config(root)?;
    let config = load_config(&config_path)?;
    let base = config_path
        .parent()
        .expect("a config file always has a parent directory");
    let Some(spec) = &config.artefact else {
        bail!(
            "{} has no [artefact] table: nothing to publish",
            config_path.display()
        );
    };
    // Worked out before packing, but a dry run only needs the files: it
    // reports what it could not work out, rather than failing on it.
    let commit = source_commit(commit, base);
    let target = source_url(base).and_then(|source| {
        let image = image_for(&source, &config.registries)?;
        Ok((source, image))
    });
    if !dry_run {
        if let Err(e) = &commit {
            bail!("{}", e);
        }
        if let Err(e) = &target {
            bail!("{}", e);
        }
    }

    let work = WorkDir(unique_temp("publish"));
    let mut layers = vec![pack_config(base, &work.0)?];
    layers.extend(pack(spec, base, &work.0)?);
    let config_blob = image_config(&layers);
    let config_digest = crate::registry::sha256_digest(&config_blob);

    match (&target, &commit) {
        (Ok((_, image)), Ok(commit)) => writeln!(
            out,
            "{} {}:{}",
            if dry_run {
                "Would publish"
            } else {
                "Publishing"
            },
            image.reference(),
            commit
        )?,
        _ => {
            writeln!(out, "Would publish:")?;
            if let Err(e) = &target {
                writeln!(out, "  image: unknown ({})", e)?;
            }
            if let Err(e) = &commit {
                writeln!(out, "  commit: unknown ({})", e)?;
            }
        }
    }
    for layer in &layers {
        writeln!(
            out,
            "  layer {}: {}, {}, {}",
            layer.name,
            crate::cache::plural(layer.files.len(), "file", "files"),
            crate::cache::human_size(layer.size),
            layer.digest
        )?;
        if dry_run {
            for file in &layer.files {
                writeln!(out, "    {}", file)?;
            }
        }
    }
    if dry_run {
        return Ok(());
    }
    let (commit, (source, image)) = (commit?, target?);
    let manifest = manifest(
        &config_digest,
        config_blob.len() as u64,
        &layers,
        &commit,
        &source,
    );

    let client = Client::new();
    if let Some(existing) = client.manifest_digest(&image, &commit)? {
        let theirs = parse_manifest(&client.manifest(&image, &existing)?)?;
        let same = theirs.layers.len() == layers.len()
            && theirs
                .layers
                .iter()
                .zip(&layers)
                .all(|(a, b)| a.digest == b.digest);
        if same {
            writeln!(out, "Already published: {}@{}", image.reference(), existing)?;
            return Ok(());
        }
        if !force {
            bail!(
                "{}:{} is already published with different files ({}). Two builds of one \
                 commit should produce the same files; pass --force to replace it",
                image.reference(),
                commit,
                existing
            );
        }
        writeln!(out, "  replacing {} (--force)", existing)?;
    }

    let config_path_blob = work.0.join("config.json");
    fs::write(&config_path_blob, &config_blob)?;
    let blobs = layers
        .iter()
        .map(|l| (l.name.as_str(), l.digest.as_str(), l.path.as_path()))
        .chain(std::iter::once((
            "config",
            config_digest.as_str(),
            config_path_blob.as_path(),
        )));
    for (name, digest, path) in blobs {
        if client.blob_exists(&image, digest, Access::Push)? {
            writeln!(out, "  {} already in the registry", name)?;
            continue;
        }
        client.upload_blob(&image, digest, path)?;
        writeln!(out, "  pushed {}", name)?;
    }
    let digest = client.put_manifest(&image, &commit, &manifest)?;
    writeln!(out, "Published {}@{}", image.reference(), digest)?;
    Ok(())
}

fn ci_var(marker: &str, value: &str) -> Option<String> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let in_ci = match marker {
        "GITLAB_CI" => var("GITLAB_CI").as_deref() == Some("true"),
        other => var(other).is_some(),
    };
    if in_ci {
        var(value)
    } else {
        None
    }
}

/// The commit being published: `--commit`, else the CI job's own, else
/// `HEAD` of the repository at `dir`.
fn source_commit(given: Option<&str>, dir: &Path) -> Result<String> {
    let commit = match given {
        Some(commit) => commit.to_string(),
        None => ci_var("GITLAB_CI", "CI_COMMIT_SHA")
            .or_else(|| ci_var("GITHUB_ACTIONS", "GITHUB_SHA"))
            .or_else(|| crate::git::query(dir, &["rev-parse", "HEAD"]))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{} is not in a git repository with a commit; pass --commit",
                    dir.display()
                )
            })?,
    };
    if !is_full_sha(&commit) {
        bail!(
            "\"{}\" is not a full commit SHA (40 or 64 hex digits)",
            commit
        );
    }
    Ok(commit.to_lowercase())
}

/// The URL of the repository being published, the one consumers declare: the
/// CI job's project, else the `origin` remote with any credentials removed.
fn source_url(dir: &Path) -> Result<String> {
    if let Some(url) = ci_var("GITLAB_CI", "CI_PROJECT_URL") {
        return Ok(url);
    }
    if let Some(repository) = ci_var("GITHUB_ACTIONS", "GITHUB_REPOSITORY") {
        let server = ci_var("GITHUB_ACTIONS", "GITHUB_SERVER_URL")
            .unwrap_or_else(|| "https://github.com".to_string());
        return Ok(format!("{}/{}", server.trim_end_matches('/'), repository));
    }
    let origin = crate::git::origin_url(dir).ok_or_else(|| {
        anyhow::anyhow!(
            "{} has no origin remote to name the published repository by",
            dir.display()
        )
    })?;
    Ok(strip_credentials(&origin))
}

/// A URL without the `user:password@` a CI checkout's remote often carries.
fn strip_credentials(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) if matches!(parsed.scheme(), "http" | "https") => {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.to_string()
        }
        _ => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LayerSpec;
    use std::os::unix::fs::PermissionsExt;

    fn is_executable(path: &Path) -> bool {
        fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    fn tempdir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gitscale-artefact-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn spec(layers: &[(&str, &[&str], &[&str])]) -> ArtefactSpec {
        ArtefactSpec {
            root: ".".into(),
            layers: layers
                .iter()
                .map(|(name, include, exclude)| LayerSpec {
                    name: name.to_string(),
                    include: include.iter().map(|s| s.to_string()).collect(),
                    exclude: exclude.iter().map(|s| s.to_string()).collect(),
                })
                .collect(),
        }
    }

    fn matches(pattern: &str, path: &str) -> bool {
        matches_path(&glob_set(&[pattern.to_string()]).unwrap(), path)
    }

    #[test]
    fn patterns_mean_what_the_docs_say() {
        assert!(matches("**", "a/b/c.txt"));
        assert!(matches("vendor/**", "vendor/x/y.js"));
        // Naming a directory includes its contents.
        assert!(matches("vendor", "vendor/x/y.js"));
        assert!(matches("*.html", "index.html"));
        assert!(!matches("*.html", "docs/index.html"));
        assert!(matches("**/*.js", "app.js"));
        assert!(matches("**/*.js", "a/b/app.js"));
        assert!(matches("assets/*.png", "assets/logo.png"));
        assert!(!matches("assets/*.png", "assets/icons/logo.png"));
        assert!(matches("{img,fonts}/**", "fonts/a.woff"));
        assert!(!matches("{img,fonts}/**", "css/a.css"));
        assert!(matches("report-?.pdf", "report-1.pdf"));
        assert!(!matches("report-?.pdf", "report-10.pdf"));
    }

    #[test]
    fn patterns_stay_inside_the_root() {
        assert!(check_pattern("../x").is_err());
        assert!(check_pattern("a/../../x").is_err());
        assert!(check_pattern("/etc/passwd").is_err());
        assert!(check_pattern("./dist").is_err());
        assert!(check_pattern("").is_err());
        assert!(check_pattern("a/[").is_err());
        assert!(check_pattern("dist/**").is_ok());
    }

    #[test]
    fn first_match_wins_and_unmatched_files_stay_behind() {
        let files: Vec<String> = ["vendor/lib.js", "app.js", "app.js.map", "README"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let groups = assign(
            &spec(&[
                ("vendor", &["vendor/**"], &[]),
                ("app", &["**/*.js", "vendor/**"], &[]),
            ]),
            &files,
        )
        .unwrap();
        assert_eq!(
            groups[0],
            ("vendor".to_string(), vec!["vendor/lib.js".to_string()])
        );
        assert_eq!(groups[1], ("app".to_string(), vec!["app.js".to_string()]));
    }

    #[test]
    fn exclude_applies_within_its_group() {
        let files: Vec<String> = ["a.js", "a.js.map", "maps/b.map"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let groups = assign(&spec(&[("app", &["**"], &["**/*.map", "maps"])]), &files).unwrap();
        assert_eq!(groups[0].1, vec!["a.js".to_string()]);
    }

    #[test]
    fn a_group_that_matches_nothing_fails() {
        let files = vec!["a.js".to_string()];
        let err = assign(
            &spec(&[("app", &["**/*.js"], &[]), ("css", &["**/*.css"], &[])]),
            &files,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("\"css\" matches no files"), "{}", err);
    }

    #[test]
    fn the_same_files_always_give_the_same_digests() {
        let dir = tempdir("repro");
        write(&dir, "src/a.txt", "alpha");
        write(&dir, "b.txt", "beta");
        let s = spec(&[("all", &["**"], &[])]);
        let first = pack(&s, &dir, &dir.join("../repro-work-1")).unwrap();
        // Touch the files: new mtimes must not change a byte.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write(&dir, "src/a.txt", "alpha");
        let second = pack(&s, &dir, &dir.join("../repro-work-2")).unwrap();
        assert_eq!(first[0].digest, second[0].digest);
        assert_eq!(first[0].diff_id, second[0].diff_id);
        assert_eq!(first[0].files, vec!["b.txt", "src/a.txt"]);
        let a = manifest("sha256:c", 1, &first, "abc", "https://example.com/a");
        let b = manifest("sha256:c", 1, &second, "abc", "https://example.com/a");
        assert_eq!(a, b);
        for d in ["../repro-work-1", "../repro-work-2", ""] {
            let _ = fs::remove_dir_all(dir.join(d));
        }
    }

    #[test]
    fn git_metadata_is_never_packed() {
        let dir = tempdir("dotgit");
        write(&dir, ".git/config", "x");
        write(&dir, "sub/.git", "gitdir: elsewhere");
        write(&dir, "sub/kept.txt", "k");
        assert_eq!(collect_files(&dir).unwrap(), vec!["sub/kept.txt"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn symlinks_are_kept_inside_the_root_and_refused_outside() {
        let dir = tempdir("links");
        write(&dir, "real/a.txt", "a");
        std::os::unix::fs::symlink("real/a.txt", dir.join("inside")).unwrap();
        assert_eq!(collect_files(&dir).unwrap(), vec!["inside", "real/a.txt"]);
        std::os::unix::fs::symlink("../../etc/passwd", dir.join("real/escape")).unwrap();
        assert!(collect_files(&dir).is_err());
        fs::remove_file(dir.join("real/escape")).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", dir.join("absolute")).unwrap();
        assert!(collect_files(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn executable_bits_survive_a_round_trip() {
        let dir = tempdir("modes");
        write(&dir, "bin/tool", "#!/bin/sh\n");
        write(&dir, "data.txt", "d");
        fs::set_permissions(dir.join("bin/tool"), fs::Permissions::from_mode(0o750)).unwrap();
        let layers = pack(&spec(&[("all", &["**"], &[])]), &dir, &dir.join("work")).unwrap();
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        extract_layer(&layers[0].path, &out, true).unwrap();
        assert!(is_executable(&out.join("bin/tool")));
        assert!(!is_executable(&out.join("data.txt")));
        assert_eq!(fs::read_to_string(out.join("data.txt")).unwrap(), "d");
        let _ = fs::remove_dir_all(&dir);
    }

    fn tar_with(dir: &Path, build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> PathBuf {
        let mut builder = tar::Builder::new(Vec::new());
        build(&mut builder);
        let path = dir.join("evil.tar");
        fs::write(&path, builder.into_inner().unwrap()).unwrap();
        path
    }

    #[test]
    fn any_name_is_the_artefacts_own() {
        // No name is gitscale's: dot files, and names gitscale once used for
        // its own markers, unpack like anything else.
        let dir = tempdir("names");
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        let archive = tar_with(&dir, |b| {
            for name in [".env", ".etag", ".gitscale-artefact", ".config/tool.toml"] {
                let mut header = tar::Header::new_gnu();
                header.set_size(1);
                header.set_mode(0o644);
                header.set_cksum();
                b.append_data(&mut header, name, &b"x"[..]).unwrap();
            }
        });
        extract_layer(&archive, &out, false).unwrap();
        for name in [".env", ".etag", ".gitscale-artefact", ".config/tool.toml"] {
            assert!(out.join(name).is_file(), "{}", name);
        }
        apply_readonly(&out).unwrap();
        assert!(fs::metadata(out.join(".config/tool.toml"))
            .unwrap()
            .permissions()
            .readonly());
        restore_writable(&out).unwrap();
        clean_files(&out).unwrap();
        assert!(is_empty_dir(&out));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hostile_archives_are_refused() {
        let dir = tempdir("hostile");
        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();

        let link_out = tar_with(&dir, |b| {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            b.append_link(&mut header, "escape", "../../outside")
                .unwrap();
        });
        assert!(extract_layer(&link_out, &out, false).is_err());

        let hardlink = tar_with(&dir, |b| {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Link);
            header.set_size(0);
            b.append_link(&mut header, "hard", "/etc/passwd").unwrap();
        });
        assert!(extract_layer(&hardlink, &out, false).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    fn entry(revision: &str) -> RepoEntry {
        RepoEntry {
            directory: "meta/app".into(),
            repo_url: "https://github.com/org/app.git".into(),
            revision: revision.into(),
            mode: crate::config::RepoMode::Artefact,
            recursive: true,
            ..Default::default()
        }
    }

    fn marker(revision: &str, commit: &str, digest: Option<&str>) -> Marker {
        Marker {
            revision: revision.into(),
            commit: commit.into(),
            digest: digest.map(str::to_string),
            image: "ghcr.io/org/app/gitscale".into(),
        }
    }

    #[test]
    fn state_reads_behind_missing_changed_and_mismatch() {
        let root = tempdir("state");
        let dest = root.join("meta/app");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("app.bin"), "x").unwrap();
        let markers = Markers::new(&root, "meta/app");
        let flags = |installed: Option<Marker>, remote: Option<Marker>, revision: &str| {
            let _ = fs::remove_file(&markers.installed);
            let _ = fs::remove_file(&markers.remote);
            if let Some(m) = installed {
                write_marker(&markers.installed, &m).unwrap();
            }
            if let Some(m) = remote {
                write_marker(&markers.remote, &m).unwrap();
            }
            state_at(&entry(revision), &dest, &markers).flags
        };
        let same = marker("main", "c1", Some("sha256:1"));
        assert!(flags(Some(same.clone()), Some(same.clone()), "main").is_empty());
        assert_eq!(
            flags(
                Some(same.clone()),
                Some(marker("main", "c2", Some("sha256:2"))),
                "main"
            ),
            vec!["behind"]
        );
        assert_eq!(
            flags(Some(same.clone()), Some(marker("main", "c2", None)), "main"),
            vec!["behind", "missing"]
        );
        assert_eq!(
            flags(
                Some(same.clone()),
                Some(marker("main", "c1", Some("sha256:9"))),
                "main"
            ),
            vec!["changed"]
        );
        // The config now names another revision: what the fetch saw for the
        // old one says nothing about the new one.
        assert_eq!(
            flags(Some(same.clone()), Some(same.clone()), "release"),
            vec!["ref-mismatch"]
        );
        // The records live outside the checkout, which holds only its files.
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);
        // A checkout somebody deleted is not installed, whatever the record says.
        fs::remove_dir_all(&dest).unwrap();
        write_marker(&markers.installed, &same).unwrap();
        assert!(state_at(&entry("main"), &dest, &markers)
            .installed
            .is_none());
        assert!(!is_downloaded(&markers, &dest));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn credentials_are_stripped_from_a_published_source_url() {
        assert_eq!(
            strip_credentials("https://gitlab-ci-token:secret@gitlab.com/group/proj.git"),
            "https://gitlab.com/group/proj.git"
        );
        assert_eq!(
            strip_credentials("git@github.com:org/app.git"),
            "git@github.com:org/app.git"
        );
    }
}

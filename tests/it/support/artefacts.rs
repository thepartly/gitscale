//! Helpers for the artefacts tests.

use super::{strip_ansi, TestEnv};
use std::path::{Path, PathBuf};
/// Two groups: a `vendor` layer that rarely changes and an `app` layer that
/// changes with every build.
pub const LAYERED: &str =
    "[[artefact.layer]]\nname = \"vendor\"\ninclude = [\"dist/vendor/**\"]\n\n\
                       [[artefact.layer]]\nname = \"app\"\ninclude = [\"dist/**\"]\n";

pub fn entry_config(env: &TestEnv, bare: &Path, revision: &str) -> String {
    format!(
        "{}[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"{}\", artefact = \"replace\" }}\n",
        env.registries(),
        bare.display(),
        revision
    )
}

pub fn read(env: &TestEnv, rel: &str) -> String {
    std::fs::read_to_string(env.playground.join(rel)).unwrap()
}

pub fn layered(env: &TestEnv, bare: &Path, app: &str) -> String {
    let (out, commit) = env.publish_with(
        bare,
        "main",
        LAYERED,
        &[("vendor/lib.js", "vendor v1"), ("app.js", app)],
        &[],
    );
    assert!(out.success, "{}{}", out.stdout, out.stderr);
    commit
}

/// The commit `ls` says `meta/app` was installed from.
pub fn installed_commit(env: &TestEnv) -> String {
    let out = env.run(&["ls", "--format", "json"]);
    let rows: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    rows.as_array()
        .unwrap()
        .iter()
        .find(|r| r["directory"] == "meta/app")
        .and_then(|r| r["artefact"]["installed"]["commit"].as_str())
        .unwrap_or_default()
        .to_string()
}

pub fn blob_downloads(env: &TestEnv) -> usize {
    env.registry().count("GET", "/blobs/sha256:")
}

pub fn status_line(env: &TestEnv) -> String {
    let out = env.run(&["ls"]);
    assert!(out.success, "{}", out.stderr);
    strip_ansi(&out.stdout)
        .lines()
        .find(|l| l.contains("meta/app"))
        .unwrap_or_default()
        .to_string()
}

/// The value `artefact show` prints on `label`'s line for the first entry.
pub fn shown(text: &str, label: &str) -> String {
    text.lines()
        .find(|l| l.trim_start().starts_with(&format!("{} ", label)))
        .map(|l| l.trim_start()[label.len()..].trim().to_string())
        .unwrap_or_default()
}

pub fn gitlab_job(env: &TestEnv) -> Vec<(&'static str, String)> {
    let addr = env.registry().addr.clone();
    let port = addr.rsplit(':').next().unwrap().to_string();
    vec![
        ("CI_JOB_TOKEN", "job-token-value".to_string()),
        ("CI_SERVER_URL", format!("http://127.0.0.1:{}", port)),
        ("CI_REGISTRY", addr),
    ]
}

/// The root's image store for `meta/app`.
pub fn image_store(env: &TestEnv) -> PathBuf {
    let dir = env.playground.join(".git/gitscale/images");
    let entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    assert_eq!(entries.len(), 1, "{:?}", entries);
    entries[0].clone()
}

/// The CI cache's image entry for `meta/app`.
pub fn cached_images(env: &TestEnv) -> PathBuf {
    let entries = env.cache_entries("images");
    assert_eq!(entries.len(), 1, "{:?}", entries);
    env.cache.join("images").join(&entries[0])
}

/// Make the image of `commit` look unused since 2000.
pub fn age(entry: &Path, commit: &str) {
    let marker = entry.join("gitscale-pins").join(commit);
    assert!(marker.is_file(), "{}", marker.display());
    let touched = std::process::Command::new("touch")
        .args(["-d", "2000-01-01", marker.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(touched.success());
}

/// A sync in a CI job: the images go to the per-user cache, not the root.
pub fn ci_pull(env: &TestEnv) {
    let out = env.run_with_env(&[("CI", "true")], &["sync"]);
    assert!(out.success, "{}{}", out.stdout, out.stderr);
}

// ---------------------------------------------------------------------------
// Crafted images: what another tool, or a hostile registry, might serve
// ---------------------------------------------------------------------------

/// A tar layer built entry by entry, with headers written as given: paths
/// with `..` or a leading `/`, links anywhere, any entry type — what a
/// hostile or merely foreign image may hold, and `tar::Builder`'s own path
/// checks would refuse to write.
pub struct Tar {
    builder: tar::Builder<Vec<u8>>,
}

impl Default for Tar {
    fn default() -> Self {
        Self::new()
    }
}

impl Tar {
    pub fn new() -> Tar {
        Tar {
            builder: tar::Builder::new(Vec::new()),
        }
    }

    fn entry(
        mut self,
        name: &str,
        kind: tar::EntryType,
        link: Option<&str>,
        data: &[u8],
        mode: u32,
    ) -> Tar {
        let mut header = tar::Header::new_gnu();
        let raw = &mut header.as_old_mut().name;
        assert!(
            name.len() < raw.len(),
            "{} is too long for a raw header",
            name
        );
        raw[..name.len()].copy_from_slice(name.as_bytes());
        header.set_entry_type(kind);
        header.set_mode(mode);
        header.set_size(data.len() as u64);
        header.set_mtime(0);
        if let Some(link) = link {
            let raw = &mut header.as_old_mut().linkname;
            raw[..link.len()].copy_from_slice(link.as_bytes());
        }
        header.set_cksum();
        self.builder.append(&header, data).unwrap();
        self
    }

    pub fn file(self, name: &str, content: &str) -> Tar {
        self.entry(
            name,
            tar::EntryType::Regular,
            None,
            content.as_bytes(),
            0o644,
        )
    }

    pub fn file_mode(self, name: &str, content: &str, mode: u32) -> Tar {
        self.entry(
            name,
            tar::EntryType::Regular,
            None,
            content.as_bytes(),
            mode,
        )
    }

    pub fn dir(self, name: &str, mode: u32) -> Tar {
        self.entry(name, tar::EntryType::Directory, None, b"", mode)
    }

    pub fn symlink(self, name: &str, target: &str) -> Tar {
        self.entry(name, tar::EntryType::Symlink, Some(target), b"", 0o777)
    }

    pub fn hardlink(self, name: &str, target: &str) -> Tar {
        self.entry(name, tar::EntryType::Link, Some(target), b"", 0o644)
    }

    pub fn fifo(self, name: &str) -> Tar {
        self.entry(name, tar::EntryType::Fifo, None, b"", 0o644)
    }

    pub fn bytes(self) -> Vec<u8> {
        self.builder.into_inner().unwrap()
    }

    pub fn gzip(self) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &self.bytes()).unwrap();
        encoder.finish().unwrap()
    }
}

/// One layer of a crafted image: its media type, bytes and title.
pub struct Layer<'a> {
    pub media_type: &'a str,
    pub bytes: Vec<u8>,
    pub title: &'a str,
}

/// A gzip layer of gitscale's own media type.
pub fn gzip_layer(title: &str, tar: Tar) -> Layer<'_> {
    Layer {
        media_type: super::registry::LAYER_GZIP,
        bytes: tar.gzip(),
        title,
    }
}

/// Put an image made of `layers` straight into the registry as the
/// artefact of `commit` in `bare` — no `artefact publish`, so nothing about
/// it has to pass gitscale's own checks. Returns the manifest's digest.
pub fn push_image(env: &TestEnv, bare: &Path, commit: &str, layers: &[Layer]) -> String {
    let registry = env.registry();
    let config = serde_json::to_vec(&serde_json::json!({
        "architecture": "unknown",
        "os": "unknown",
        "config": {},
        "rootfs": { "type": "layers", "diff_ids": [] },
    }))
    .unwrap();
    let config_digest = registry.put_blob(&config);
    let descriptors: Vec<serde_json::Value> = layers
        .iter()
        .map(|layer| {
            let digest = registry.put_blob(&layer.bytes);
            serde_json::json!({
                "mediaType": layer.media_type,
                "digest": digest,
                "size": layer.bytes.len(),
                "annotations": { "org.opencontainers.image.title": layer.title },
            })
        })
        .collect();
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": super::registry::MANIFEST_TYPE,
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": config_digest,
            "size": config.len(),
        },
        "layers": descriptors,
    }))
    .unwrap();
    registry.put_manifest(
        &env.image(bare),
        commit,
        &manifest,
        super::registry::MANIFEST_TYPE,
    )
}

/// The head of `branch` in `bare`.
pub fn tip(bare: &Path, branch: &str) -> String {
    super::git_stdout(bare, &["rev-parse", branch])
}

/// The layer digests of the image published for `commit`, in order.
pub fn layer_digests(env: &TestEnv, bare: &Path, commit: &str) -> Vec<String> {
    let manifest = env.registry().manifest(&env.image(bare), commit).unwrap();
    manifest["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["digest"].as_str().unwrap().to_string())
        .collect()
}

/// Commit a symlink `link` -> `target` to `branch` of `bare`.
pub fn commit_symlink(
    env: &TestEnv,
    bare: &Path,
    branch: &str,
    link: &str,
    target: &str,
) -> String {
    let work = env.repos_remote.join("symlink-tmp");
    let _ = std::fs::remove_dir_all(&work);
    super::run_git_pub(
        &env.repos_remote,
        &[
            "clone",
            "-q",
            "--branch",
            branch,
            bare.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    super::run_git_pub(&work, &["config", "user.email", "t@t"]);
    super::run_git_pub(&work, &["config", "user.name", "T"]);
    let path = work.join(link);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, &path).unwrap();
    super::run_git_pub(&work, &["add", "-A"]);
    super::run_git_pub(&work, &["commit", "-q", "-m", "link"]);
    super::run_git_pub(&work, &["push", "-q", "origin", branch]);
    let commit = super::git_stdout(&work, &["rev-parse", "HEAD"]);
    let _ = std::fs::remove_dir_all(&work);
    commit
}

/// An overlay entry for `bare` at `revision`.
pub fn overlay_config(env: &TestEnv, bare: &Path, revision: &str) -> String {
    format!(
        "{}[repos]\n\"meta/app\" = {{ url = \"{}\", revision = \"{}\", artefact = \"overlay\" }}\n",
        env.registries(),
        bare.display(),
        revision
    )
}

/// Make every file and directory under `dir` writable again, so a test can
/// delete a checkout gitscale made read-only.
pub fn make_tree_writable(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(listing) = std::fs::read_dir(dir) else {
        return;
    };
    if let Ok(meta) = std::fs::symlink_metadata(dir) {
        if meta.is_dir() {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
        }
    }
    for found in listing.flatten() {
        let path = found.path();
        let Ok(kind) = found.file_type() else {
            continue;
        };
        if kind.is_dir() {
            make_tree_writable(&path);
        } else if kind.is_file() {
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode | 0o200));
        }
    }
}

/// Every file name anywhere under `dir`, for spotting leftovers.
pub fn all_names(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(listing) = std::fs::read_dir(dir) {
        for found in listing.flatten() {
            names.push(found.file_name().to_string_lossy().into_owned());
            if found.file_type().is_ok_and(|k| k.is_dir()) {
                names.extend(all_names(&found.path()));
            }
        }
    }
    names
}

/// The gitscale binary run in `dir`, with `vars` on top of the test's own
/// cache — for commands whose `-C` must be some directory other than the
/// playground, such as a producer's checkout.
pub fn run_bin_in(
    env: &TestEnv,
    dir: &Path,
    vars: &[(&str, &str)],
    args: &[&str],
) -> super::CliOutput {
    let mut full: Vec<&str> = Vec::new();
    let nested = matches!(args.first(), Some(&"artefact") | Some(&"cache")) && args.len() > 1;
    let split = if nested { 2 } else { 1 };
    full.extend_from_slice(&args[..split]);
    full.extend_from_slice(&["-C", dir.to_str().unwrap()]);
    full.extend_from_slice(&args[split..]);
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_gitscale"));
    cmd.args(&full).env("GITSCALE_CACHE_DIR", &env.cache);
    for (name, value) in vars {
        cmd.env(name, value);
    }
    let output = cmd.output().expect("failed to run the gitscale binary");
    super::CliOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        success: output.status.success(),
    }
}

/// A fresh producer checkout of `bare`'s `main` with a build in `dist/`
/// (ignored) and a config that publishes it to this environment's registry.
pub fn producer_with_build(env: &TestEnv, bare: &Path) -> PathBuf {
    let producer = env.producer(bare, "main");
    std::fs::create_dir_all(producer.join("dist")).unwrap();
    std::fs::write(producer.join("dist/app.bin"), "built").unwrap();
    std::fs::write(producer.join(".git/info/exclude"), "/dist/\n").unwrap();
    std::fs::write(
        producer.join(".gitscale.toml"),
        format!("{}[artefact]\ninclude = [\"dist/**\"]\n", env.registries()),
    )
    .unwrap();
    producer
}

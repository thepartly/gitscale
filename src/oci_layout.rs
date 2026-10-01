//! An OCI image layout on disk — `oci-layout`, `index.json`,
//! `blobs/sha256/<hex>` — holding one repository's artefacts in the object
//! cache.
//!
//! Every blob is named by its own digest, so the store cannot hand out the
//! wrong bytes without noticing: a blob is checked against its name whenever
//! it is read, and one that fails is deleted and downloaded again. Nothing
//! borrows from it — a checkout gets copies — so anything here may be deleted
//! at any time without breaking a workspace.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::registry::{file_digest, valid_digest, MANIFEST_MEDIA_TYPE};

const LAYOUT_FILE: &str = "oci-layout";
const INDEX_FILE: &str = "index.json";
/// The annotation the OCI layout spec uses for a manifest's tag.
const REF_NAME: &str = "org.opencontainers.image.ref.name";
const REVISION: &str = "org.opencontainers.image.revision";

pub struct Layout {
    path: PathBuf,
}

/// One manifest the layout holds: the image for one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub commit: String,
    pub digest: String,
}

#[derive(Serialize, Deserialize, Default)]
struct Index {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(rename = "mediaType", default)]
    media_type: String,
    #[serde(default)]
    manifests: Vec<IndexEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct IndexEntry {
    #[serde(rename = "mediaType")]
    media_type: String,
    digest: String,
    size: u64,
    #[serde(default)]
    annotations: std::collections::BTreeMap<String, String>,
}

/// The parts of a manifest the store follows: which blobs it needs.
#[derive(Deserialize)]
struct ManifestRefs {
    config: Option<Descriptor>,
    #[serde(default)]
    layers: Vec<Descriptor>,
}

#[derive(Deserialize)]
struct Descriptor {
    digest: String,
}

impl Layout {
    pub fn new(path: PathBuf) -> Self {
        Layout { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether `path` holds a layout, as against being missing or half made.
    pub fn exists(path: &Path) -> bool {
        path.join(LAYOUT_FILE).is_file()
    }

    /// Create the layout if it is not there yet.
    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(self.path.join("blobs/sha256"))
            .with_context(|| format!("cannot create {}", self.path.display()))?;
        if !self.path.join(INDEX_FILE).is_file() {
            self.write_index(&Index {
                schema_version: 2,
                media_type: "application/vnd.oci.image.index.v1+json".to_string(),
                manifests: Vec::new(),
            })?;
        }
        // Written last: it is what says the layout is usable.
        if !Layout::exists(&self.path) {
            fs::write(
                self.path.join(LAYOUT_FILE),
                br#"{"imageLayoutVersion":"1.0.0"}"#,
            )?;
        }
        Ok(())
    }

    /// Where the blob `digest` lives. Refuses anything that is not a
    /// well-formed digest, since it becomes a file name.
    pub fn blob_path(&self, digest: &str) -> Result<PathBuf> {
        if !valid_digest(digest) {
            anyhow::bail!("not a sha256 digest: {}", digest);
        }
        Ok(self
            .path
            .join("blobs/sha256")
            .join(digest.trim_start_matches("sha256:")))
    }

    /// The blob `digest`, if the layout holds it intact. One whose content no
    /// longer matches its name is deleted, so the caller downloads it again.
    pub fn verified_blob(&self, digest: &str) -> Option<PathBuf> {
        let path = self.blob_path(digest).ok()?;
        if !path.is_file() {
            return None;
        }
        match file_digest(&path) {
            Ok(actual) if actual == digest => Some(path),
            _ => {
                let _ = fs::remove_file(&path);
                None
            }
        }
    }

    /// Record that `commit`'s image is the manifest `digest`, replacing what
    /// the layout said before — a forced re-publish moves the commit.
    pub fn record(&self, commit: &str, digest: &str, size: u64) -> Result<()> {
        let mut index = self.read_index();
        index
            .manifests
            .retain(|m| m.annotations.get(REF_NAME).map(String::as_str) != Some(commit));
        index.manifests.push(IndexEntry {
            media_type: MANIFEST_MEDIA_TYPE.to_string(),
            digest: digest.to_string(),
            size,
            annotations: [
                (REF_NAME.to_string(), commit.to_string()),
                (REVISION.to_string(), commit.to_string()),
            ]
            .into_iter()
            .collect(),
        });
        self.write_index(&index)
    }

    /// Every commit the layout holds an image for.
    pub fn held(&self) -> Vec<Held> {
        self.read_index()
            .manifests
            .into_iter()
            .filter_map(|m| {
                Some(Held {
                    commit: m.annotations.get(REF_NAME)?.clone(),
                    digest: m.digest,
                })
            })
            .collect()
    }

    /// Forget the images of `commits`. Their blobs go at the next [`gc`].
    ///
    /// [`gc`]: Layout::gc
    pub fn forget(&self, commits: &[String]) -> Result<()> {
        let mut index = self.read_index();
        index.manifests.retain(|m| {
            m.annotations
                .get(REF_NAME)
                .is_none_or(|commit| !commits.contains(commit))
        });
        self.write_index(&index)
    }

    /// Delete every blob no held manifest needs, and anything a download
    /// left half-written. Returns the bytes freed.
    pub fn gc(&self) -> Result<u64> {
        let mut wanted: HashSet<String> = HashSet::new();
        for held in self.held() {
            wanted.insert(held.digest.clone());
            let Ok(path) = self.blob_path(&held.digest) else {
                continue;
            };
            let Ok(text) = fs::read(&path) else {
                continue;
            };
            if let Ok(refs) = serde_json::from_slice::<ManifestRefs>(&text) {
                wanted.extend(refs.config.map(|c| c.digest));
                wanted.extend(refs.layers.into_iter().map(|l| l.digest));
            }
        }
        let mut freed = 0;
        let Ok(listing) = fs::read_dir(self.path.join("blobs/sha256")) else {
            return Ok(0);
        };
        for found in listing.flatten() {
            let name = found.file_name().to_string_lossy().into_owned();
            if wanted.contains(&format!("sha256:{}", name)) {
                continue;
            }
            freed += found.metadata().map(|m| m.len()).unwrap_or(0);
            fs::remove_file(found.path())
                .with_context(|| format!("cannot remove {}", found.path().display()))?;
        }
        Ok(freed)
    }

    /// Check every blob against its name and delete those that fail.
    /// Returns how many were deleted.
    pub fn verify(&self) -> Result<usize> {
        let mut removed = 0;
        let Ok(listing) = fs::read_dir(self.path.join("blobs/sha256")) else {
            return Ok(0);
        };
        for found in listing.flatten() {
            let name = found.file_name().to_string_lossy().into_owned();
            let intact = file_digest(&found.path())
                .map(|d| d == format!("sha256:{}", name))
                .unwrap_or(false);
            if !intact {
                fs::remove_file(found.path())?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn read_index(&self) -> Index {
        fs::read(self.path.join(INDEX_FILE))
            .ok()
            .and_then(|text| serde_json::from_slice(&text).ok())
            .unwrap_or_default()
    }

    fn write_index(&self, index: &Index) -> Result<()> {
        // Through a temporary file: a reader never sees half an index.
        let tmp = self
            .path
            .join(format!(".{}.{}", INDEX_FILE, std::process::id()));
        fs::write(&tmp, serde_json::to_vec_pretty(index)?)?;
        fs::rename(&tmp, self.path.join(INDEX_FILE))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::sha256_digest;

    fn layout(name: &str) -> Layout {
        let path =
            std::env::temp_dir().join(format!("gitscale-layout-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&path);
        let layout = Layout::new(path);
        layout.ensure().unwrap();
        layout
    }

    fn put(layout: &Layout, bytes: &[u8]) -> String {
        let digest = sha256_digest(bytes);
        fs::write(layout.blob_path(&digest).unwrap(), bytes).unwrap();
        digest
    }

    fn manifest(layout: &Layout, layers: &[&str]) -> (String, u64) {
        let config = put(layout, b"{}");
        let body = serde_json::json!({
            "schemaVersion": 2,
            "config": {"digest": config, "size": 2},
            "layers": layers.iter().map(|d| serde_json::json!({"digest": d, "size": 1})).collect::<Vec<_>>(),
        })
        .to_string();
        let digest = put(layout, body.as_bytes());
        (digest, body.len() as u64)
    }

    #[test]
    fn gc_keeps_what_held_manifests_need_and_nothing_else() {
        let layout = layout("gc");
        let shared = put(&layout, b"vendor");
        let old_app = put(&layout, b"app v1");
        let new_app = put(&layout, b"app v2");
        let (m1, s1) = manifest(&layout, &[&shared, &old_app]);
        let (m2, s2) = manifest(&layout, &[&shared, &new_app]);
        layout.record("c1", &m1, s1).unwrap();
        layout.record("c2", &m2, s2).unwrap();
        assert_eq!(layout.gc().unwrap(), 0);

        layout.forget(&["c1".to_string()]).unwrap();
        assert!(layout.gc().unwrap() > 0);
        // The layer both commits shared survives the one that went.
        assert!(layout.verified_blob(&shared).is_some());
        assert!(layout.verified_blob(&new_app).is_some());
        assert!(layout.verified_blob(&old_app).is_none());
        assert!(layout.verified_blob(&m1).is_none());
        let _ = fs::remove_dir_all(layout.path());
    }

    #[test]
    fn a_forced_republish_moves_the_commit_and_frees_the_old_image() {
        let layout = layout("republish");
        let v1 = put(&layout, b"build one");
        let v2 = put(&layout, b"build two");
        let (m1, s1) = manifest(&layout, &[&v1]);
        let (m2, s2) = manifest(&layout, &[&v2]);
        layout.record("c1", &m1, s1).unwrap();
        layout.record("c1", &m2, s2).unwrap();
        assert_eq!(
            layout.held(),
            vec![Held {
                commit: "c1".into(),
                digest: m2.clone()
            }]
        );
        layout.gc().unwrap();
        assert!(layout.verified_blob(&v1).is_none());
        assert!(layout.verified_blob(&v2).is_some());
        let _ = fs::remove_dir_all(layout.path());
    }

    #[test]
    fn a_damaged_blob_is_dropped_rather_than_served() {
        let layout = layout("damaged");
        let digest = put(&layout, b"good bytes");
        fs::write(layout.blob_path(&digest).unwrap(), b"bad bytes").unwrap();
        assert!(layout.verified_blob(&digest).is_none());
        assert!(!layout.blob_path(&digest).unwrap().exists());

        let digest = put(&layout, b"good bytes");
        fs::write(layout.blob_path(&digest).unwrap(), b"rot").unwrap();
        assert_eq!(layout.verify().unwrap(), 1);
        let _ = fs::remove_dir_all(layout.path());
    }

    #[test]
    fn a_digest_never_becomes_a_path_outside_the_layout() {
        let layout = layout("paths");
        assert!(layout.blob_path("sha256:../../../etc/passwd").is_err());
        assert!(layout.blob_path("md5:abc").is_err());
        let _ = fs::remove_dir_all(layout.path());
    }
}

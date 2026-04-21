use anyhow::{bail, Result};
use chrono::Utc;
use hmac::{Hmac, Mac};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use url::Url;

use crate::urls::{extract_hostname, extract_owner_repo};

#[derive(Debug, Clone)]
pub struct HeadResult {
    pub exists: bool,
    pub etag: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn object_url(storage_url: &str, repo_url: &str, revision: &str) -> Result<String> {
    let hostname = extract_hostname(repo_url)?;
    let (owner, repo) = extract_owner_repo(repo_url)?;
    let safe_rev = if revision.is_empty() {
        "_default".to_string()
    } else {
        revision.replace('/', "_")
    };
    let base = storage_url.trim_end_matches('/');
    Ok(format!(
        "{}/{}/{}/{}/{}.tar.gz",
        base, hostname, owner, repo, safe_rev
    ))
}

pub fn head_object(url: &str) -> Result<HeadResult> {
    if is_local(url) {
        local_head(url)
    } else if is_gcs(url) {
        gcs_head(url)
    } else {
        s3_head(url)
    }
}

pub fn get_object(url: &str) -> Result<Option<(Vec<u8>, String)>> {
    if is_local(url) {
        local_get(url)
    } else if is_gcs(url) {
        gcs_get(url)
    } else {
        s3_get(url)
    }
}

pub fn put_object(url: &str, body: &[u8]) -> Result<String> {
    if is_local(url) {
        local_put(url, body)
    } else if is_gcs(url) {
        gcs_put(url, body)
    } else {
        s3_put(url, body)
    }
}

// ---------------------------------------------------------------------------
// Artefact operations
// ---------------------------------------------------------------------------

pub fn clone_artefact(
    storage_url: &str,
    repo_url: &str,
    revision: &str,
    dest: &Path,
) -> Result<bool> {
    let url = object_url(storage_url, repo_url, revision)?;
    let Some((body, etag)) = get_object(&url)? else {
        return Ok(false);
    };
    fs::create_dir_all(dest)?;
    fs::write(dest.join(".etag"), &etag)?;
    fs::write(dest.join(".etag-remote"), &etag)?;
    extract_artefact(&body, dest)?;
    apply_artefact_readonly(dest)?;
    Ok(true)
}

pub fn fetch_artefact(
    storage_url: &str,
    repo_url: &str,
    revision: &str,
    dest: &Path,
) -> Result<HeadResult> {
    let url = object_url(storage_url, repo_url, revision)?;
    let result = head_object(&url)?;
    if result.exists {
        fs::create_dir_all(dest)?;
        fs::write(dest.join(".etag-remote"), &result.etag)?;
    }
    Ok(result)
}

pub fn pull_artefact(
    storage_url: &str,
    repo_url: &str,
    revision: &str,
    dest: &Path,
) -> Result<bool> {
    let url = object_url(storage_url, repo_url, revision)?;
    let hr = head_object(&url)?;
    if !hr.exists {
        return Ok(false);
    }
    fs::create_dir_all(dest)?;
    fs::write(dest.join(".etag-remote"), &hr.etag)?;

    // Check if local is already up to date
    let local_etag_file = dest.join(".etag");
    if local_etag_file.is_file() {
        let local_etag = fs::read_to_string(&local_etag_file)?.trim().to_string();
        if local_etag == hr.etag {
            return Ok(true); // up to date
        }
    }

    let Some((body, etag)) = get_object(&url)? else {
        return Ok(false);
    };
    restore_artefact_writable(dest)?;
    clean_artefact_files(dest)?;
    extract_artefact(&body, dest)?;
    fs::write(dest.join(".etag"), &etag)?;
    apply_artefact_readonly(dest)?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Artefact helpers
// ---------------------------------------------------------------------------

fn extract_artefact(body: &[u8], dest: &Path) -> Result<()> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let decoder = GzDecoder::new(body);
    let mut archive = Archive::new(decoder);
    let dest_canonical = dest.canonicalize()?;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_path = entry.path()?.to_path_buf();
        let resolved = dest
            .join(&entry_path)
            .canonicalize()
            .unwrap_or_else(|_| dest.join(&entry_path));

        // Security: reject paths that escape dest
        if !resolved.starts_with(&dest_canonical) {
            bail!(
                "artefact archive contains unsafe path: {}",
                entry_path.display()
            );
        }
        entry.unpack_in(dest)?;
    }
    Ok(())
}

fn apply_artefact_readonly(dest: &Path) -> Result<()> {
    for entry in fs::read_dir(dest)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && !entry.file_name().to_string_lossy().starts_with('.') {
            let meta = fs::metadata(&path)?;
            let mut perms = meta.permissions();
            perms.set_mode(perms.mode() & !(0o222));
            fs::set_permissions(&path, perms)?;
        }
    }
    Ok(())
}

fn restore_artefact_writable(dest: &Path) -> Result<()> {
    for entry in fs::read_dir(dest)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && !entry.file_name().to_string_lossy().starts_with('.') {
            let meta = fs::metadata(&path)?;
            let mut perms = meta.permissions();
            perms.set_mode(perms.mode() | 0o200);
            fs::set_permissions(&path, perms)?;
        }
    }
    Ok(())
}

fn clean_artefact_files(dest: &Path) -> Result<()> {
    for entry in fs::read_dir(dest)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Backend detection
// ---------------------------------------------------------------------------

fn is_local(url: &str) -> bool {
    url.starts_with("file://") || url.starts_with('/') || url.starts_with("./")
}

fn is_gcs(url: &str) -> bool {
    if let Ok(parsed) = Url::parse(url) {
        if let Some(host) = parsed.host_str() {
            return host == "storage.googleapis.com" || host.ends_with(".storage.googleapis.com");
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Local filesystem
// ---------------------------------------------------------------------------

fn local_fs_path(url: &str) -> PathBuf {
    PathBuf::from(url.strip_prefix("file://").unwrap_or(url))
}

fn local_content_etag(content: &[u8]) -> String {
    format!("\"{}\"", hex::encode(Sha256::digest(content)))
}

fn local_head(url: &str) -> Result<HeadResult> {
    let path = local_fs_path(url);
    if !path.is_file() {
        return Ok(HeadResult {
            exists: false,
            etag: String::new(),
        });
    }
    let content = fs::read(&path)?;
    let etag = local_content_etag(&content);
    Ok(HeadResult { exists: true, etag })
}

fn local_get(url: &str) -> Result<Option<(Vec<u8>, String)>> {
    let path = local_fs_path(url);
    if !path.is_file() {
        return Ok(None);
    }
    let content = fs::read(&path)?;
    let etag = local_content_etag(&content);
    Ok(Some((content, etag)))
}

fn local_put(url: &str, body: &[u8]) -> Result<String> {
    let path = local_fs_path(url);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, body)?;
    Ok(local_content_etag(body))
}

// ---------------------------------------------------------------------------
// GCS (Bearer token)
// ---------------------------------------------------------------------------

fn gcs_token() -> Result<String> {
    if let Ok(token) = std::env::var("GOOGLE_TOKEN") {
        if !token.is_empty() {
            return Ok(token);
        }
    }
    if let Ok(token) = std::env::var("GCLOUD_ACCESS_TOKEN") {
        if !token.is_empty() {
            return Ok(token);
        }
    }
    bail!("No GCS token found. Set GOOGLE_TOKEN or GCLOUD_ACCESS_TOKEN.")
}

fn gcs_head(url: &str) -> Result<HeadResult> {
    let token = gcs_token()?;
    let client = reqwest::blocking::Client::new();
    let resp = client
        .head(url)
        .header("Authorization", format!("Bearer {}", token))
        .timeout(std::time::Duration::from_secs(30))
        .send()?;
    if resp.status().as_u16() == 404 {
        return Ok(HeadResult {
            exists: false,
            etag: String::new(),
        });
    }
    check_storage_response(&resp, "HEAD")?;
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    Ok(HeadResult { exists: true, etag })
}

fn gcs_get(url: &str) -> Result<Option<(Vec<u8>, String)>> {
    let token = gcs_token()?;
    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(url)
        .header("Authorization", format!("Bearer {}", token))
        .timeout(std::time::Duration::from_secs(30))
        .send()?;
    if resp.status().as_u16() == 404 {
        return Ok(None);
    }
    check_storage_response(&resp, "GET")?;
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.bytes()?.to_vec();
    Ok(Some((body, etag)))
}

fn gcs_put(url: &str, body: &[u8]) -> Result<String> {
    let token = gcs_token()?;
    let client = reqwest::blocking::Client::new();
    let resp = client
        .put(url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .body(body.to_vec())
        .timeout(std::time::Duration::from_secs(30))
        .send()?;
    check_storage_response(&resp, "PUT")?;
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    Ok(etag)
}

// ---------------------------------------------------------------------------
// S3 (AWS Signature V4)
// ---------------------------------------------------------------------------

fn s3_credentials() -> Result<(String, String)> {
    let access_key = std::env::var("AWS_ACCESS_KEY_ID").unwrap_or_default();
    let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY").unwrap_or_default();
    if access_key.is_empty() || secret_key.is_empty() {
        bail!("No S3 credentials found. Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY.");
    }
    Ok((access_key, secret_key))
}

fn s3_region(url: &str) -> String {
    if let Ok(region) = std::env::var("AWS_DEFAULT_REGION") {
        if !region.is_empty() {
            return region;
        }
    }
    if let Ok(parsed) = Url::parse(url) {
        if let Some(host) = parsed.host_str() {
            let re = Regex::new(r"s3[.\-]([a-z0-9-]+)\.amazonaws\.com").unwrap();
            if let Some(caps) = re.captures(host) {
                let region = &caps[1];
                if region != "amazonaws" {
                    return region.to_string();
                }
            }
        }
    }
    "us-east-1".to_string()
}

type HmacSha256 = Hmac<Sha256>;

fn hmac_sign(key: &[u8], msg: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC key length");
    mac.update(msg.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn s3_sign(
    method: &str,
    url: &str,
    body: &[u8],
    access_key: &str,
    secret_key: &str,
    region: &str,
) -> Vec<(String, String)> {
    let parsed = Url::parse(url).expect("valid URL");
    let host = parsed.host_str().unwrap_or("");
    let path = parsed.path();

    let now = Utc::now();
    let datestamp = now.format("%Y%m%d").to_string();
    let amzdate = now.format("%Y%m%dT%H%M%SZ").to_string();

    let payload_hash = hex::encode(Sha256::digest(body));

    // Canonical headers (sorted)
    let canonical_headers = format!(
        "host:{}\nx-amz-content-sha256:{}\nx-amz-date:{}\n",
        host, payload_hash, amzdate
    );
    let signed_headers = "host;x-amz-content-sha256;x-amz-date";

    // Canonical request
    let canonical_request = format!(
        "{}\n{}\n\n{}\n{}\n{}",
        method, path, canonical_headers, signed_headers, payload_hash
    );

    // String to sign
    let scope = format!("{}/{}/s3/aws4_request", datestamp, region);
    let canonical_hash = hex::encode(Sha256::digest(canonical_request.as_bytes()));
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amzdate, scope, canonical_hash
    );

    // Signing key
    let k_date = hmac_sign(format!("AWS4{}", secret_key).as_bytes(), &datestamp);
    let k_region = hmac_sign(&k_date, region);
    let k_service = hmac_sign(&k_region, "s3");
    let k_signing = hmac_sign(&k_service, "aws4_request");

    let signature = hex::encode(hmac_sign(&k_signing, &string_to_sign));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        access_key, scope, signed_headers, signature
    );

    vec![
        ("Authorization".to_string(), authorization),
        ("x-amz-date".to_string(), amzdate),
        ("x-amz-content-sha256".to_string(), payload_hash),
        ("Host".to_string(), host.to_string()),
    ]
}

fn s3_head(url: &str) -> Result<HeadResult> {
    let (access_key, secret_key) = s3_credentials()?;
    let region = s3_region(url);
    let headers = s3_sign("HEAD", url, b"", &access_key, &secret_key, &region);

    let client = reqwest::blocking::Client::new();
    let mut req = client.head(url).timeout(std::time::Duration::from_secs(30));
    for (k, v) in &headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send()?;
    if resp.status().as_u16() == 404 {
        return Ok(HeadResult {
            exists: false,
            etag: String::new(),
        });
    }
    check_storage_response(&resp, "HEAD")?;
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    Ok(HeadResult { exists: true, etag })
}

fn s3_get(url: &str) -> Result<Option<(Vec<u8>, String)>> {
    let (access_key, secret_key) = s3_credentials()?;
    let region = s3_region(url);
    let headers = s3_sign("GET", url, b"", &access_key, &secret_key, &region);

    let client = reqwest::blocking::Client::new();
    let mut req = client.get(url).timeout(std::time::Duration::from_secs(30));
    for (k, v) in &headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send()?;
    if resp.status().as_u16() == 404 {
        return Ok(None);
    }
    check_storage_response(&resp, "GET")?;
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.bytes()?.to_vec();
    Ok(Some((body, etag)))
}

fn s3_put(url: &str, body: &[u8]) -> Result<String> {
    let (access_key, secret_key) = s3_credentials()?;
    let region = s3_region(url);
    let headers = s3_sign("PUT", url, body, &access_key, &secret_key, &region);

    let client = reqwest::blocking::Client::new();
    let mut req = client
        .put(url)
        .body(body.to_vec())
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(30));
    for (k, v) in &headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send()?;
    check_storage_response(&resp, "PUT")?;
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    Ok(etag)
}

// ---------------------------------------------------------------------------
// Shared
// ---------------------------------------------------------------------------

fn check_storage_response(resp: &reqwest::blocking::Response, method: &str) -> Result<()> {
    let status = resp.status().as_u16();
    if matches!(status, 200 | 201 | 204) {
        return Ok(());
    }
    bail!(
        "Storage {} failed ({}): response status {}",
        method,
        status,
        status
    );
}

//! OCI registries: where a repository's artefacts live, and the client that
//! reads and writes them over the Distribution API.
//!
//! An artefact is an ordinary OCI image — one gzip tar layer per group of
//! files, tagged with the full commit it was built from — so any registry
//! works: GitLab's, GHCR, ECR, Artifact Registry, Harbor, `registry:2`.
//!
//! Credentials follow the same two rules as git's in [`crate::ci`]: the CI job
//! token goes only to the registry the CI server owns, and to a token service
//! on that server or on the registry itself; and no secret is ever kept
//! anywhere but in memory for the length of one command.

use anyhow::{bail, Context, Result};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::ci::{CiAuth, Forge};

pub const MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
pub const CONFIG_MEDIA_TYPE: &str = "application/vnd.oci.image.config.v1+json";
pub const LAYER_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
/// What a manifest request accepts: an OCI manifest, or the Docker one older
/// tooling writes, which describes the same thing.
const ACCEPT_MANIFESTS: &str = "application/vnd.oci.image.manifest.v1+json, \
                                application/vnd.docker.distribution.manifest.v2+json";
/// Appended to every repository path, so artefacts never share a name with
/// the images a project publishes itself — and a cleanup policy can exempt
/// them with one pattern.
pub const IMAGE_SUFFIX: &str = "gitscale";

/// Small requests: manifests, existence checks, token exchanges.
const SHORT_TIMEOUT: Duration = Duration::from_secs(30);
/// A layer of a large artefact over a slow link.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(1800);

/// Where one repository's artefacts live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// `host[:port]`.
    pub registry: String,
    /// The repository inside the registry, ending in [`IMAGE_SUFFIX`].
    pub repository: String,
    /// Spoken to over plain HTTP: configured with `http://`, or on this
    /// machine.
    pub plain_http: bool,
}

impl Image {
    /// `registry/repository`, as `docker pull` and the docs spell it.
    pub fn reference(&self) -> String {
        format!("{}/{}", self.registry, self.repository)
    }

    fn base_url(&self) -> String {
        let scheme = if self.plain_http || is_local(&self.registry) {
            "http"
        } else {
            "https"
        };
        format!("{}://{}", scheme, self.registry)
    }

    fn url(&self, kind: &str, reference: &str) -> String {
        format!(
            "{}/v2/{}/{}/{}",
            self.base_url(),
            self.repository,
            kind,
            reference
        )
    }
}

/// Plain HTTP is allowed for a registry on this machine and nowhere else —
/// the allowance every OCI tool makes, and what tests run against.
pub fn is_local(registry: &str) -> bool {
    matches!(host_of(registry), "localhost" | "127.0.0.1" | "[::1]")
}

/// `host[:port]` without the port; an IPv6 literal keeps its brackets.
fn host_of(registry: &str) -> &str {
    if registry.starts_with('[') {
        return match registry.find(']') {
            Some(end) => &registry[..=end],
            None => registry,
        };
    }
    registry.split(':').next().unwrap_or(registry)
}

/// The image `repo_url`'s artefacts are published as.
///
/// The consumer and `artefact publish` both call this, from an entry's URL
/// and from the producer's own remote, so the two always agree.
pub fn image_for(repo_url: &str, registries: &BTreeMap<String, String>) -> Result<Image> {
    image_for_with(repo_url, registries, crate::ci::active())
}

/// [`image_for`] with the CI environment passed in, for tests.
///
/// First match wins:
/// 1. `[registries]`: a key with a `/` is a URL prefix (the longest
///    that matches), anything else a host;
/// 2. the CI server's own host maps to the registry it owns — `CI_REGISTRY`
///    on GitLab, `ghcr.io` or `containers.<host>` on GitHub;
/// 3. `github.com` → `ghcr.io`, `gitlab.com` → `registry.gitlab.com`.
pub fn image_for_with(
    repo_url: &str,
    registries: &BTreeMap<String, String>,
    ci: Option<&CiAuth>,
) -> Result<Image> {
    let host = crate::urls::extract_hostname(repo_url)
        .ok()
        .map(|h| h.to_lowercase());

    let by_prefix = registries
        .iter()
        .filter(|(key, _)| key.contains('/'))
        .filter_map(|(key, registry)| {
            strip_url_prefix(repo_url, key).map(|rest| (key.len(), registry, rest))
        })
        .max_by_key(|(len, _, _)| *len);

    let (base, path) = if let Some((_, registry, rest)) = by_prefix {
        (registry.clone(), rest.to_string())
    } else {
        let path = || crate::urls::extract_path(repo_url);
        let by_host = host.as_deref().and_then(|host| {
            registries
                .iter()
                .find(|(key, _)| !key.contains('/') && key.eq_ignore_ascii_case(host))
                .map(|(_, registry)| registry.clone())
        });
        let by_ci = ci
            .filter(|auth| host.as_deref() == Some(auth.host.as_str()))
            .and_then(|auth| auth.registry.clone());
        let builtin = match host.as_deref() {
            Some("github.com") => Some("ghcr.io".to_string()),
            Some("gitlab.com") => Some("registry.gitlab.com".to_string()),
            _ => None,
        };
        match by_host.or(by_ci).or(builtin) {
            Some(registry) => (registry, path()?),
            None => bail!(
                "no registry is known for {}: add one under [registries], e.g. \"{}\" = \"registry.{}\"",
                repo_url,
                host.as_deref().unwrap_or("<host or URL prefix>"),
                host.as_deref().unwrap_or("example.com")
            ),
        }
    };

    // An explicit opt-in to plain HTTP, for a registry without TLS.
    let (plain_http, base) = match base.strip_prefix("http://") {
        Some(rest) => (true, rest.to_string()),
        None => (false, base),
    };
    let (registry, namespace) = match base.split_once('/') {
        Some((registry, namespace)) => (registry, namespace.trim_matches('/')),
        None => (base.as_str(), ""),
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path).to_lowercase();
    let mut parts: Vec<&str> = Vec::new();
    if !namespace.is_empty() {
        parts.push(namespace);
    }
    parts.push(&path);
    parts.push(IMAGE_SUFFIX);
    let repository = parts.join("/").to_lowercase();
    for component in repository.split('/') {
        if !valid_component(component) {
            bail!(
                "{} maps to the image {}/{}, but \"{}\" is not a valid registry path component",
                repo_url,
                registry,
                repository,
                component
            );
        }
    }
    Ok(Image {
        registry: registry.to_lowercase(),
        repository,
        plain_http,
    })
}

/// `url` with the configured prefix `key` taken off the front, if it starts
/// with it on a path boundary.
fn strip_url_prefix<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let rest = url.strip_prefix(key)?;
    if key.ends_with('/') || rest.starts_with('/') {
        let rest = rest.trim_start_matches('/');
        (!rest.is_empty()).then_some(rest)
    } else {
        None
    }
}

/// A path component the Distribution spec allows:
/// `[a-z0-9]+((\.|_|__|-+)[a-z0-9]+)*`.
fn valid_component(component: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^[a-z0-9]+(?:(?:\.|_|__|-+)[a-z0-9]+)*$").unwrap())
        .is_match(component)
}

/// `sha256:` and 64 lowercase hex digits — checked before a digest from a
/// manifest is ever used as a file name.
pub fn valid_digest(digest: &str) -> bool {
    digest.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
    })
}

pub fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

/// The SHA-256 digest of a file, read in chunks.
pub fn file_digest(path: &Path) -> Result<String> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

/// Whether a request reads or writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Pull,
    Push,
}

impl Access {
    fn scope(self, image: &Image) -> String {
        let actions = match self {
            Access::Pull => "pull",
            Access::Push => "pull,push",
        };
        format!("repository:{}:{}", image.repository, actions)
    }

    fn verb(self) -> &'static str {
        match self {
            Access::Pull => "read",
            Access::Push => "publish to",
        }
    }
}

/// A Distribution API client for one command. Tokens a registry hands out are
/// kept for the life of the client, per registry and scope.
///
/// The HTTP clients behind it are built on first use: each one starts a
/// runtime thread, and most commands in a workspace without artefacts never
/// need one.
pub struct Client {
    http: OnceLock<reqwest::blocking::Client>,
    /// For registries on this machine: no proxy, which would otherwise be
    /// asked to reach 127.0.0.1 on our behalf.
    local: OnceLock<reqwest::blocking::Client>,
    tokens: Mutex<HashMap<String, String>>,
}

type Request = reqwest::blocking::RequestBuilder;
type Response = reqwest::blocking::Response;

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        Client {
            http: OnceLock::new(),
            local: OnceLock::new(),
            tokens: Mutex::new(HashMap::new()),
        }
    }

    fn http(&self, local: bool) -> &reqwest::blocking::Client {
        let cell = if local { &self.local } else { &self.http };
        cell.get_or_init(|| {
            let mut builder = reqwest::blocking::Client::builder()
                .connect_timeout(SHORT_TIMEOUT)
                .timeout(SHORT_TIMEOUT)
                .user_agent(concat!("gitscale/", env!("CARGO_PKG_VERSION")));
            if local {
                builder = builder.no_proxy();
            }
            // Building fails only when no TLS backend can start, which no
            // request could have survived either; the default client fails
            // the same way, at the first request, with the error attached.
            builder
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new())
        })
    }

    fn http_for(&self, image: &Image) -> &reqwest::blocking::Client {
        self.http(is_local(&image.registry))
    }

    /// Send a request built by `make`, answering the registry's 401 challenge
    /// once. A response that is still 401 is returned for the caller to
    /// report.
    fn send(
        &self,
        image: &Image,
        access: Access,
        make: &dyn Fn(&reqwest::blocking::Client) -> Request,
    ) -> Result<Response> {
        let key = format!("{}|{}", image.registry, access.scope(image));
        let http = self.http_for(image);
        let attempt = |header: Option<&str>| {
            let mut request = make(http);
            if let Some(header) = header {
                request = request.header(reqwest::header::AUTHORIZATION, header);
            }
            request
                .send()
                .with_context(|| format!("cannot reach the registry {}", image.registry))
        };
        let cached = self.tokens.lock().unwrap().get(&key).cloned();
        let response = attempt(cached.as_deref())?;
        if response.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Ok(response);
        }
        let challenge = response
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let Some(header) = self.authorize(image, access, &challenge)? else {
            return Ok(response);
        };
        self.tokens.lock().unwrap().insert(key, header.clone());
        attempt(Some(&header))
    }

    /// The `Authorization` header that answers `challenge`, or `None` when
    /// there is nothing to answer it with.
    fn authorize(&self, image: &Image, access: Access, challenge: &str) -> Result<Option<String>> {
        let Some(challenge) = Challenge::parse(challenge) else {
            return Ok(None);
        };
        match challenge {
            Challenge::Basic => {
                // Never a credential in the clear, except to this machine.
                if image.plain_http && !is_local(&image.registry) {
                    return Ok(None);
                }
                Ok(credentials(&image.registry, None).and_then(|c| c.basic()))
            }
            Challenge::Bearer {
                realm,
                service,
                scope,
            } => {
                let realm_url = url::Url::parse(&realm).with_context(|| {
                    format!(
                        "{} named an invalid token service: {}",
                        image.registry, realm
                    )
                })?;
                let realm_host = realm_url.host_str().unwrap_or_default().to_lowercase();
                let realm_authority = match realm_url.port() {
                    Some(port) => format!("{}:{}", realm_host, port),
                    None => realm_host.clone(),
                };
                // Credentials travel only over TLS, unless both ends are on
                // this machine.
                let secure = realm_url.scheme() == "https" || is_local(&realm_authority);
                let credential = if secure {
                    credentials(&image.registry, Some(&realm_host))
                } else {
                    None
                };
                let scope = scope.unwrap_or_else(|| access.scope(image));
                let http = self.http(is_local(&realm_authority));
                let mut query: Vec<(&str, &str)> = vec![("scope", &scope)];
                if let Some(service) = &service {
                    query.push(("service", service));
                }
                let request = match &credential {
                    Some(Credential::IdentityToken(token)) => {
                        let mut form = vec![
                            ("grant_type", "refresh_token"),
                            ("refresh_token", token.as_str()),
                            ("client_id", "gitscale"),
                            ("scope", scope.as_str()),
                        ];
                        if let Some(service) = &service {
                            form.push(("service", service));
                        }
                        http.post(realm_url.as_str()).form(&form)
                    }
                    Some(Credential::Basic { username, password }) => http
                        .get(realm_url.as_str())
                        .query(&query)
                        .basic_auth(username, Some(password)),
                    None => http.get(realm_url.as_str()).query(&query),
                };
                let response = request
                    .send()
                    .with_context(|| format!("cannot reach the token service {}", realm))?;
                if !response.status().is_success() {
                    // Refused credentials surface as the 401 the caller
                    // reports, with its hint.
                    return Ok(None);
                }
                let body: serde_json::Value = serde_json::from_slice(&response.bytes()?)
                    .with_context(|| format!("{} returned an unreadable token", realm))?;
                let token = body
                    .get("token")
                    .or_else(|| body.get("access_token"))
                    .and_then(|t| t.as_str())
                    .filter(|t| !t.is_empty());
                Ok(token.map(|t| format!("Bearer {}", t)))
            }
        }
    }

    /// The digest of the manifest `reference` (a tag or digest) names, or
    /// `None` when the registry has no such manifest.
    pub fn manifest_digest(&self, image: &Image, reference: &str) -> Result<Option<String>> {
        let url = image.url("manifests", reference);
        let response = self.send(image, Access::Pull, &|http| {
            http.head(&url)
                .header(reqwest::header::ACCEPT, ACCEPT_MANIFESTS)
        })?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = check(response, image, Access::Pull)?;
        if let Some(digest) = response
            .headers()
            .get("docker-content-digest")
            .and_then(|v| v.to_str().ok())
            .filter(|d| valid_digest(d))
        {
            return Ok(Some(digest.to_string()));
        }
        // Not every registry sends the digest on HEAD; the body says it.
        let response = check(
            self.send(image, Access::Pull, &|http| {
                http.get(&url)
                    .header(reqwest::header::ACCEPT, ACCEPT_MANIFESTS)
            })?,
            image,
            Access::Pull,
        )?;
        Ok(Some(sha256_digest(&response.bytes()?)))
    }

    /// Every tag in the image's repository, following the registry's
    /// pagination. An empty list when the repository does not exist yet.
    pub fn tags(&self, image: &Image) -> Result<Vec<String>> {
        let mut tags = Vec::new();
        let mut next = Some(format!("{}?n=1000", image.url("tags", "list")));
        // A registry that keeps naming a next page is not given forever.
        let mut pages = 0;
        while let Some(url) = next.take() {
            pages += 1;
            if pages > 1000 {
                bail!("{} kept paginating its tag list", image.registry);
            }
            let response = self.send(image, Access::Pull, &|http| http.get(&url))?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                break;
            }
            let response = check(response, image, Access::Pull)?;
            next = response
                .headers()
                .get(reqwest::header::LINK)
                .and_then(|v| v.to_str().ok())
                .and_then(next_link)
                .map(|link| url::Url::parse(&image.base_url()).and_then(|base| base.join(&link)))
                .transpose()
                .with_context(|| format!("{} gave an invalid next page", image.registry))?
                .map(|u| u.to_string());
            let body: serde_json::Value = serde_json::from_slice(&response.bytes()?)
                .with_context(|| format!("{} returned an unreadable tag list", image.registry))?;
            if let Some(page) = body.get("tags").and_then(|t| t.as_array()) {
                tags.extend(page.iter().filter_map(|t| t.as_str()).map(str::to_string));
            }
        }
        Ok(tags)
    }

    /// The manifest with `digest`, checked against it.
    pub fn manifest(&self, image: &Image, digest: &str) -> Result<Vec<u8>> {
        let url = image.url("manifests", digest);
        let response = check(
            self.send(image, Access::Pull, &|http| {
                http.get(&url)
                    .header(reqwest::header::ACCEPT, ACCEPT_MANIFESTS)
            })?,
            image,
            Access::Pull,
        )?;
        let body = response.bytes()?.to_vec();
        if sha256_digest(&body) != digest {
            bail!(
                "{} served a manifest that does not match its digest {}",
                image.reference(),
                digest
            );
        }
        Ok(body)
    }

    pub fn blob_exists(&self, image: &Image, digest: &str, access: Access) -> Result<bool> {
        let url = image.url("blobs", digest);
        let response = self.send(image, access, &|http| http.head(&url))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }
        check(response, image, access)?;
        Ok(true)
    }

    /// Download the blob `digest` to `dest`, checked against the digest
    /// before it is moved into place: nothing that fails the check is ever
    /// left where a reader would find it.
    pub fn download_blob(&self, image: &Image, digest: &str, dest: &Path) -> Result<()> {
        let url = image.url("blobs", digest);
        let mut response = check(
            self.send(image, Access::Pull, &|http| {
                http.get(&url).timeout(TRANSFER_TIMEOUT)
            })?,
            image,
            Access::Pull,
        )?;
        let partial = partial_path(dest);
        let result = (|| -> Result<()> {
            let file = std::fs::File::create(&partial)
                .with_context(|| format!("cannot create {}", partial.display()))?;
            let mut writer = HashingWriter::new(file);
            std::io::copy(&mut response, &mut writer).with_context(|| {
                format!("download of {} from {} failed", digest, image.reference())
            })?;
            let (file, got, _) = writer.finish();
            file.sync_all()?;
            if got != digest {
                bail!(
                    "{} served a blob that does not match its digest {}",
                    image.reference(),
                    digest
                );
            }
            std::fs::rename(&partial, dest)
                .with_context(|| format!("cannot move {} into place", dest.display()))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
        result
    }

    /// Upload the file `source` as the blob `digest`, in one request.
    pub fn upload_blob(&self, image: &Image, digest: &str, source: &Path) -> Result<()> {
        let start = image.url("blobs", "uploads/");
        let response = check(
            self.send(image, Access::Push, &|http| {
                http.post(&start)
                    .header(reqwest::header::CONTENT_LENGTH, "0")
            })?,
            image,
            Access::Push,
        )?;
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                anyhow::anyhow!("{} started an upload without a Location", image.registry)
            })?;
        // Registries answer with a path, an absolute URL, or either with a
        // query of their own already on it.
        let mut target = url::Url::parse(&image.base_url())?
            .join(location)
            .with_context(|| {
                format!(
                    "{} gave an invalid upload location: {}",
                    image.registry, location
                )
            })?;
        target.query_pairs_mut().append_pair("digest", digest);
        let size = std::fs::metadata(source)?.len();
        let response = check(
            self.send(image, Access::Push, &|http| {
                let body = std::fs::File::open(source).map(reqwest::blocking::Body::from);
                let request = http
                    .put(target.as_str())
                    .timeout(TRANSFER_TIMEOUT)
                    .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                    .header(reqwest::header::CONTENT_LENGTH, size.to_string());
                match body {
                    Ok(body) => request.body(body),
                    Err(_) => request,
                }
            })?,
            image,
            Access::Push,
        )?;
        drop(response);
        Ok(())
    }

    /// Put `manifest` under `tag`; returns its digest.
    pub fn put_manifest(&self, image: &Image, tag: &str, manifest: &[u8]) -> Result<String> {
        let url = image.url("manifests", tag);
        check(
            self.send(image, Access::Push, &|http| {
                http.put(&url)
                    .header(reqwest::header::CONTENT_TYPE, MANIFEST_MEDIA_TYPE)
                    .body(manifest.to_vec())
            })?,
            image,
            Access::Push,
        )?;
        Ok(sha256_digest(manifest))
    }
}

/// The target of a `Link: <…>; rel="next"` header.
fn next_link(header: &str) -> Option<String> {
    header.split(',').find_map(|part| {
        let (target, params) = part.split_once(';')?;
        let is_next = params.split(';').any(|p| {
            p.trim()
                .replace(' ', "")
                .eq_ignore_ascii_case("rel=\"next\"")
        });
        let target = target.trim().strip_prefix('<')?.strip_suffix('>')?;
        is_next.then(|| target.to_string())
    })
}

/// Where a download is written before it has been checked.
fn partial_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    dest.with_file_name(format!(".{}.partial-{}", name, std::process::id()))
}

/// A response that succeeded, or an error that says what to do about it.
fn check(response: Response, image: &Image, access: Access) -> Result<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        bail!(
            "{} refused to let gitscale {} {} ({})\nhint: {}",
            image.registry,
            access.verb(),
            image.repository,
            status.as_u16(),
            access_hint(image, access)
        );
    }
    let body: String = response
        .text()
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect();
    bail!(
        "{} answered {} for {}{}",
        image.registry,
        status.as_u16(),
        image.repository,
        if body.trim().is_empty() {
            String::new()
        } else {
            format!(": {}", body.trim())
        }
    )
}

/// What to tell the user when a registry refuses: whose permission is
/// missing depends on which credential was offered.
fn access_hint(image: &Image, access: Access) -> String {
    if let Some(auth) = crate::ci::active().filter(|a| a.trusts_registry(&image.registry, None)) {
        return match (auth.forge, access) {
            (Forge::GitLab, Access::Pull) => format!(
                "the GitLab job token ({}) was refused. Add this project to the producing \
                 project's Settings -> CI/CD -> 'Job token permissions' allowlist.",
                auth.token_env
            ),
            (Forge::GitLab, Access::Push) => format!(
                "the GitLab job token ({}) can publish only to its own project's registry.",
                auth.token_env
            ),
            (Forge::GitHub, Access::Pull) => format!(
                "the GitHub Actions token ({}) was refused. Give this repository read access \
                 under the package's Settings -> 'Manage Actions access', and add \
                 `permissions: packages: read` to the workflow.",
                auth.token_env
            ),
            (Forge::GitHub, Access::Push) => format!(
                "the GitHub Actions token ({}) needs `permissions: packages: write` in the \
                 workflow, and write access to the package if it already exists.",
                auth.token_env
            ),
        };
    }
    format!(
        "log in once with `docker login {}` (or podman/oras login) using a token that can {} packages",
        image.registry,
        match access {
            Access::Pull => "read",
            Access::Push => "write",
        }
    )
}

/// A registry's `WWW-Authenticate` challenge.
#[derive(Debug, PartialEq, Eq)]
enum Challenge {
    Basic,
    Bearer {
        realm: String,
        service: Option<String>,
        scope: Option<String>,
    },
}

impl Challenge {
    fn parse(header: &str) -> Option<Challenge> {
        let header = header.trim();
        let (scheme, rest) = header.split_once(' ').unwrap_or((header, ""));
        if scheme.eq_ignore_ascii_case("basic") {
            return Some(Challenge::Basic);
        }
        if !scheme.eq_ignore_ascii_case("bearer") {
            return None;
        }
        let params = parse_params(rest);
        Some(Challenge::Bearer {
            realm: params.get("realm")?.clone(),
            service: params.get("service").cloned(),
            scope: params.get("scope").cloned(),
        })
    }
}

/// `key="value", key=value` pairs, with commas allowed inside quotes — a
/// scope with several actions is `repository:x:pull,push`.
fn parse_params(text: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| *c == ',' || c.is_whitespace()) {
            chars.next();
        }
        let key: String = chars
            .by_ref()
            .take_while(|c| *c != '=')
            .collect::<String>()
            .trim()
            .to_lowercase();
        if key.is_empty() {
            break;
        }
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(escaped) = chars.next() {
                            value.push(escaped);
                        }
                    }
                    '"' => break,
                    c => value.push(c),
                }
            }
        } else {
            while let Some(c) = chars.peek() {
                if *c == ',' {
                    break;
                }
                value.push(*c);
                chars.next();
            }
            value = value.trim().to_string();
        }
        params.insert(key, value);
    }
    params
}

/// A credential for one registry.
#[derive(Clone, PartialEq, Eq)]
pub enum Credential {
    Basic {
        username: String,
        password: String,
    },
    /// An OAuth2 refresh token, as `docker login` stores for some registries.
    IdentityToken(String),
}

impl std::fmt::Debug for Credential {
    // Never print a secret, even in a debug dump.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Credential::Basic { username, .. } => write!(f, "Basic({}, ***)", username),
            Credential::IdentityToken(_) => write!(f, "IdentityToken(***)"),
        }
    }
}

impl Credential {
    fn basic(&self) -> Option<String> {
        match self {
            Credential::Basic { username, password } => Some(format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", username, password))
            )),
            Credential::IdentityToken(_) => None,
        }
    }
}

/// The credential to offer `registry`, whose token service is at
/// `realm_host` (`None` for a Basic challenge, answered by the registry
/// itself). In CI the job token, if this is the CI server's own registry;
/// otherwise whatever `docker login` or `podman login` stored for it.
fn credentials(registry: &str, realm_host: Option<&str>) -> Option<Credential> {
    if let Some(auth) = crate::ci::active() {
        if auth.trusts_registry(registry, realm_host) {
            if let Some(token) = auth.token() {
                return Some(Credential::Basic {
                    username: auth.username.clone(),
                    password: token,
                });
            }
        }
    }
    stored_credential(registry)
}

/// What `docker login` (or `podman login`, `oras login`) stored for
/// `registry`. Sent to whatever token service that registry names, as docker
/// itself does: the credential was stored for this registry, so it is this
/// registry's to direct.
pub fn stored_credential(registry: &str) -> Option<Credential> {
    auth_files()
        .into_iter()
        .find_map(|file| from_auth_file(&file, registry))
}

/// Docker's config, then Podman's auth file, in the order those tools read
/// them. `REGISTRY_AUTH_FILE` replaces Podman's default locations, as it does
/// for Podman itself.
fn auth_files() -> Vec<PathBuf> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let mut files = Vec::new();
    match var("DOCKER_CONFIG") {
        Some(dir) => files.push(PathBuf::from(dir).join("config.json")),
        None => {
            if let Some(home) = var("HOME") {
                files.push(PathBuf::from(home).join(".docker/config.json"));
            }
        }
    }
    match var("REGISTRY_AUTH_FILE") {
        Some(file) => files.push(PathBuf::from(file)),
        None => {
            if let Some(runtime) = var("XDG_RUNTIME_DIR") {
                files.push(PathBuf::from(runtime).join("containers/auth.json"));
            }
            if let Some(home) = var("HOME") {
                files.push(PathBuf::from(home).join(".config/containers/auth.json"));
            }
        }
    }
    files
}

fn from_auth_file(path: &Path, registry: &str) -> Option<Credential> {
    let text = std::fs::read_to_string(path).ok()?;
    let config: serde_json::Value = serde_json::from_str(&text).ok()?;
    let helper = config
        .get("credHelpers")
        .and_then(|h| h.get(registry))
        .or_else(|| config.get("credsStore"))
        .and_then(|h| h.as_str());
    if let Some(credential) = helper.and_then(|helper| from_helper(helper, registry)) {
        return Some(credential);
    }
    let auths = config.get("auths")?.as_object()?;
    auths
        .iter()
        .find(|(key, _)| auth_key_host(key).eq_ignore_ascii_case(registry))
        .and_then(|(_, entry)| {
            if let Some(token) = entry.get("identitytoken").and_then(|t| t.as_str()) {
                if !token.is_empty() {
                    return Some(Credential::IdentityToken(token.to_string()));
                }
            }
            let encoded = entry.get("auth")?.as_str()?;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .ok()?;
            let decoded = String::from_utf8(decoded).ok()?;
            let (username, password) = decoded.split_once(':')?;
            Some(Credential::Basic {
                username: username.to_string(),
                password: password.to_string(),
            })
        })
}

/// An `auths` key reduced to `host[:port]`: they are written as a bare host,
/// a URL, or Docker Hub's `https://index.docker.io/v1/`.
fn auth_key_host(key: &str) -> &str {
    let key = key
        .strip_prefix("https://")
        .or_else(|| key.strip_prefix("http://"))
        .unwrap_or(key);
    key.split('/').next().unwrap_or(key)
}

/// Ask a Docker credential helper (`docker-credential-<name> get`).
fn from_helper(name: &str, registry: &str) -> Option<Credential> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return None;
    }
    let mut child = std::process::Command::new(format!("docker-credential-{}", name))
        .arg("get")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(registry.as_bytes()).ok()?;
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let answer: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let username = answer.get("Username")?.as_str()?.to_string();
    let secret = answer.get("Secret")?.as_str()?.to_string();
    if secret.is_empty() {
        return None;
    }
    if username == "<token>" {
        return Some(Credential::IdentityToken(secret));
    }
    Some(Credential::Basic {
        username,
        password: secret,
    })
}

/// A writer that hashes and counts what passes through it.
pub struct HashingWriter<W: Write> {
    inner: W,
    hasher: Sha256,
    written: u64,
}

impl<W: Write> HashingWriter<W> {
    pub fn new(inner: W) -> Self {
        HashingWriter {
            inner,
            hasher: Sha256::new(),
            written: 0,
        }
    }

    /// The inner writer, the `sha256:` digest of everything written, and its
    /// length.
    pub fn finish(self) -> (W, String, u64) {
        (
            self.inner,
            format!("sha256:{}", hex::encode(self.hasher.finalize())),
            self.written,
        )
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    #[test]
    fn forges_map_to_their_registries() {
        let image = image_for_with("https://github.com/Org/Tools.git", &none(), None).unwrap();
        assert_eq!(image.reference(), "ghcr.io/org/tools/gitscale");
        let image = image_for_with("git@gitlab.com:org/sub/frontend.git", &none(), None).unwrap();
        assert_eq!(
            image.reference(),
            "registry.gitlab.com/org/sub/frontend/gitscale"
        );
    }

    #[test]
    fn transports_of_one_repo_share_an_image() {
        let a = image_for_with("git@github.com:org/app.git", &none(), None).unwrap();
        let b = image_for_with("https://github.com/org/app", &none(), None).unwrap();
        let c = image_for_with("ssh://git@github.com/org/app.git", &none(), None).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn an_unknown_host_names_the_setting_to_add() {
        let err = image_for_with("https://git.corp.example/team/svc.git", &none(), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("[registries]"), "{}", err);
        assert!(err.contains("git.corp.example"), "{}", err);
    }

    #[test]
    fn configured_hosts_and_prefixes_win() {
        let registries = BTreeMap::from([
            (
                "git.corp.example".to_string(),
                "registry.corp.example".to_string(),
            ),
            (
                "github.com".to_string(),
                "mirror.example:5000/ns".to_string(),
            ),
            ("/srv/repos/".to_string(), "127.0.0.1:5000".to_string()),
            ("/srv/repos/deep/".to_string(), "127.0.0.1:6000".to_string()),
        ]);
        let image = |url| image_for_with(url, &registries, None).unwrap().reference();
        assert_eq!(
            image("https://git.corp.example/team/svc.git"),
            "registry.corp.example/team/svc/gitscale"
        );
        assert_eq!(
            image("https://github.com/org/app"),
            "mirror.example:5000/ns/org/app/gitscale"
        );
        assert_eq!(image("/srv/repos/app.git"), "127.0.0.1:5000/app/gitscale");
        // The longest prefix wins.
        assert_eq!(
            image("/srv/repos/deep/lib.git"),
            "127.0.0.1:6000/lib/gitscale"
        );
        // A prefix matches only on a path boundary.
        assert!(image_for_with("/srv/repository.git", &registries, None).is_err());
    }

    #[test]
    fn the_ci_server_maps_to_the_registry_it_owns() {
        let auth = CiAuth::from_map(&HashMap::from([
            ("CI_JOB_TOKEN", "t"),
            ("CI_SERVER_URL", "https://gitlab.corp.example"),
            ("CI_REGISTRY", "registry.corp.example:5050"),
        ]))
        .unwrap();
        let image =
            image_for_with("git@gitlab.corp.example:team/svc.git", &none(), Some(&auth)).unwrap();
        assert_eq!(
            image.reference(),
            "registry.corp.example:5050/team/svc/gitscale"
        );
        // Another host is not the CI server's to map.
        assert!(image_for_with("https://other.example/a/b", &none(), Some(&auth)).is_err());
    }

    #[test]
    fn names_the_registry_cannot_hold_are_refused() {
        let registries = BTreeMap::from([("/srv/".to_string(), "127.0.0.1:5000".to_string())]);
        assert!(image_for_with("/srv/My Repo.git", &registries, None).is_err());
        assert!(image_for_with("/srv/a..b.git", &registries, None).is_err());
    }

    #[test]
    fn only_this_machine_gets_plain_http() {
        assert!(is_local("127.0.0.1:5000"));
        assert!(is_local("localhost"));
        assert!(is_local("[::1]:5000"));
        assert!(!is_local("registry.example.com"));
        assert!(!is_local("localhost.evil.example"));
        let image = |registry: &str| Image {
            registry: registry.into(),
            repository: "a/gitscale".into(),
            plain_http: false,
        };
        assert!(image("127.0.0.1:5000").base_url().starts_with("http://"));
        assert!(image("ghcr.io").base_url().starts_with("https://"));
        // Anything else gets plain HTTP only when configured with http://.
        let registries =
            BTreeMap::from([("/srv/".to_string(), "http://registry:5000/ns".to_string())]);
        let opted = image_for_with("/srv/app.git", &registries, None).unwrap();
        assert!(opted.plain_http);
        assert_eq!(opted.reference(), "registry:5000/ns/app/gitscale");
        assert!(opted.base_url().starts_with("http://registry:5000"));
    }

    #[test]
    fn challenges_parse_with_commas_in_quotes() {
        assert_eq!(
            Challenge::parse(
                r#"Bearer realm="https://gitlab.com/jwt/auth",service="container_registry",scope="repository:a/b:pull,push""#
            ),
            Some(Challenge::Bearer {
                realm: "https://gitlab.com/jwt/auth".into(),
                service: Some("container_registry".into()),
                scope: Some("repository:a/b:pull,push".into()),
            })
        );
        assert_eq!(
            Challenge::parse(r#"Basic realm="Registry Realm""#),
            Some(Challenge::Basic)
        );
        assert_eq!(
            Challenge::parse(r#"bearer realm="https://ghcr.io/token""#),
            Some(Challenge::Bearer {
                realm: "https://ghcr.io/token".into(),
                service: None,
                scope: None,
            })
        );
        assert_eq!(Challenge::parse("Negotiate"), None);
        assert_eq!(Challenge::parse(r#"Bearer service="x""#), None);
    }

    #[test]
    fn the_next_page_is_read_from_the_link_header() {
        assert_eq!(
            next_link(r#"</v2/a/tags/list?n=2&last=b>; rel="next""#).as_deref(),
            Some("/v2/a/tags/list?n=2&last=b")
        );
        assert_eq!(
            next_link(r#"<https://r.example/x>; rel="prev", <https://r.example/y>; rel="next""#)
                .as_deref(),
            Some("https://r.example/y")
        );
        assert_eq!(next_link(r#"</x>; rel="prev""#), None);
    }

    #[test]
    fn digests_are_checked_before_use_as_names() {
        assert!(valid_digest(&format!("sha256:{}", "a".repeat(64))));
        assert!(!valid_digest(&format!("sha256:{}", "A".repeat(64))));
        assert!(!valid_digest("sha256:../../etc/passwd"));
        assert!(!valid_digest(&format!("sha512:{}", "a".repeat(64))));
    }

    #[test]
    fn auth_file_keys_reduce_to_the_registry() {
        assert_eq!(auth_key_host("ghcr.io"), "ghcr.io");
        assert_eq!(
            auth_key_host("https://index.docker.io/v1/"),
            "index.docker.io"
        );
        assert_eq!(auth_key_host("http://127.0.0.1:5000"), "127.0.0.1:5000");
    }

    #[test]
    fn stored_logins_are_read_from_an_auth_file() {
        let dir = std::env::temp_dir().join(format!("gitscale-auth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.json");
        let auth = base64::engine::general_purpose::STANDARD.encode("dev:s3cret");
        std::fs::write(
            &file,
            format!(
                r#"{{"auths": {{"https://ghcr.io": {{"auth": "{}"}}, "registry.example": {{"identitytoken": "rt"}}}}}}"#,
                auth
            ),
        )
        .unwrap();
        assert_eq!(
            from_auth_file(&file, "ghcr.io"),
            Some(Credential::Basic {
                username: "dev".into(),
                password: "s3cret".into()
            })
        );
        assert_eq!(
            from_auth_file(&file, "registry.example"),
            Some(Credential::IdentityToken("rt".into()))
        );
        assert_eq!(from_auth_file(&file, "other.example"), None);
        let _ = std::fs::remove_dir_all(&dir);
        // And no secret reaches a debug dump.
        let shown = format!(
            "{:?}",
            Credential::Basic {
                username: "dev".into(),
                password: "s3cret".into()
            }
        );
        assert!(!shown.contains("s3cret"));
    }
}

//! A fake OCI registry for the tests: the slice of the Distribution API
//! gitscale calls, served from memory on 127.0.0.1, with a request log and
//! switches for the failures a real registry can produce.
//!
//! Every switch is off by default, and a registry with none of them set
//! behaves exactly as it always has: Bearer auth through its own `/token`
//! when asked for, relative upload locations, two tags a page.
#![allow(dead_code)]

use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

pub const MANIFEST_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
pub const INDEX_TYPE: &str = "application/vnd.oci.image.index.v1+json";
pub const LAYER_GZIP: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
pub const LAYER_TAR: &str = "application/vnd.oci.image.layer.v1.tar";
pub const DOCKER_LAYER_GZIP: &str = "application/vnd.docker.image.rootfs.diff.tar.gzip";
/// The token the fake token service hands out for good credentials.
pub const TOKEN: &str = "fake-registry-token";

/// Require bearer tokens, issued by `/token` for `user:password`.
#[derive(Clone)]
pub struct Auth {
    pub user: String,
    pub password: String,
    /// The host named in the challenge's realm: `127.0.0.1` for the
    /// registry's own token service, anything else to play a token service
    /// somewhere else.
    pub realm_host: String,
}

/// One request, as the registry saw it.
#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    /// The query string, as sent.
    pub query: String,
    /// The `Authorization` header, verbatim.
    pub authorization: Option<String>,
}

/// A failure to answer requests with, instead of serving them.
#[derive(Clone)]
struct Failure {
    method: String,
    part: String,
    status: u16,
    body: String,
    /// Put the request's headers in the body, as some proxies and debug
    /// endpoints do.
    echo_headers: bool,
    /// Only once the request has passed authentication.
    after_auth: bool,
}

#[derive(Default)]
struct State {
    /// (repository, tag or digest) -> manifest bytes.
    manifests: HashMap<(String, String), Vec<u8>>,
    /// Manifest digest -> the media type to serve it with, when it is not
    /// an image manifest.
    manifest_types: HashMap<String, String>,
    /// (repository, tag) -> the digest to claim for it instead of the real
    /// one.
    digest_lies: HashMap<(String, String), String>,
    blobs: HashMap<String, Vec<u8>>,
    uploads: HashMap<String, String>,
    next_upload: u64,
    log: Vec<String>,
    seen: Vec<Seen>,
    auth: Option<Auth>,
    /// Answer every `/v2/` request with this status.
    refuse: Option<u16>,
    /// Serve blobs with their bytes changed.
    corrupt: bool,
    /// Serve these blobs with their bytes changed.
    corrupt_digests: HashSet<String>,
    /// Serve these blobs cut off half way, the connection closed early.
    truncate_digests: HashSet<String>,
    /// Leave `Docker-Content-Digest` off manifest responses.
    omit_manifest_digest: bool,
    /// Challenge with `Basic` rather than `Bearer`.
    basic: bool,
    /// The realm to name in a Bearer challenge, instead of this registry's
    /// own `/token`.
    realm: Option<String>,
    /// The field the token service puts the token in.
    token_field: Option<String>,
    /// Refresh tokens the token service exchanges for a token (`POST`).
    refresh_tokens: Vec<String>,
    /// Redirect blob downloads to this base URL.
    blob_redirect: Option<String>,
    /// Hand out upload locations on this base URL.
    upload_base: Option<String>,
    /// Accept a blob upload whatever started it: for a registry that only
    /// plays the host an upload location points at.
    accept_any_upload: bool,
    /// Name next pages of the tag list on this base URL.
    tags_next_base: Option<String>,
    /// Name a next page of the tag list forever.
    endless_tags: bool,
    failures: Vec<Failure>,
}

pub struct FakeRegistry {
    pub addr: String,
    state: Arc<Mutex<State>>,
}

impl FakeRegistry {
    pub fn start() -> FakeRegistry {
        FakeRegistry::start_on("127.0.0.1")
    }

    /// A registry listening on `host` — `127.0.0.2`, say, for a host
    /// gitscale does not count as this machine, though it is.
    pub fn start_on(host: &str) -> FakeRegistry {
        let listener = TcpListener::bind(format!("{}:0", host)).expect("bind the fake registry");
        let addr = listener.local_addr().unwrap().to_string();
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = shared.clone();
                std::thread::spawn(move || {
                    let _ = serve(stream, &state, port);
                });
            }
        });
        FakeRegistry { addr, state }
    }

    pub fn require_auth(&self, auth: Auth) {
        self.state.lock().unwrap().auth = Some(auth);
    }

    pub fn refuse_with(&self, status: u16) {
        self.state.lock().unwrap().refuse = Some(status);
    }

    pub fn corrupt_blobs(&self) {
        self.state.lock().unwrap().corrupt = true;
    }

    /// Serve the blob `digest` with its bytes changed.
    pub fn corrupt_blob(&self, digest: &str) {
        self.state
            .lock()
            .unwrap()
            .corrupt_digests
            .insert(digest.to_string());
    }

    /// Serve the blob `digest` cut off half way.
    pub fn truncate_blob(&self, digest: &str) {
        self.state
            .lock()
            .unwrap()
            .truncate_digests
            .insert(digest.to_string());
    }

    /// Leave `Docker-Content-Digest` off manifest responses, as some
    /// registries do on `HEAD`.
    pub fn omit_manifest_digest(&self) {
        self.state.lock().unwrap().omit_manifest_digest = true;
    }

    /// Challenge with `Basic` for the `require_auth` credentials.
    pub fn use_basic_auth(&self) {
        self.state.lock().unwrap().basic = true;
    }

    /// Name `realm` in Bearer challenges rather than this registry's own
    /// token service.
    pub fn set_realm(&self, realm: &str) {
        self.state.lock().unwrap().realm = Some(realm.to_string());
    }

    /// Hand tokens out in `field` (`access_token`, say) instead of `token`.
    pub fn token_field(&self, field: &str) {
        self.state.lock().unwrap().token_field = Some(field.to_string());
    }

    /// Exchange `refresh_token` for a token, as the OAuth2 flow behind a
    /// `docker login` identity token does.
    pub fn accept_refresh_token(&self, refresh_token: &str) {
        self.state
            .lock()
            .unwrap()
            .refresh_tokens
            .push(refresh_token.to_string());
    }

    /// Redirect every blob download to the same path under `base`
    /// (`http://host:port`).
    pub fn redirect_blobs_to(&self, base: &str) {
        self.state.lock().unwrap().blob_redirect = Some(base.to_string());
    }

    /// Start uploads with an absolute location under `base`.
    pub fn upload_to(&self, base: &str) {
        self.state.lock().unwrap().upload_base = Some(base.to_string());
    }

    /// Accept blob uploads this registry never started.
    pub fn accept_any_upload(&self) {
        self.state.lock().unwrap().accept_any_upload = true;
    }

    /// Name the next page of a tag list as an absolute URL under `base`.
    pub fn tags_next_page_on(&self, base: &str) {
        self.state.lock().unwrap().tags_next_base = Some(base.to_string());
    }

    /// Name a next page of every tag list, forever.
    pub fn endless_tags(&self) {
        self.state.lock().unwrap().endless_tags = true;
    }

    /// Answer `method` requests whose path contains `part` with `status`
    /// and `body`.
    pub fn fail(&self, method: &str, part: &str, status: u16, body: &str) {
        self.state.lock().unwrap().failures.push(Failure {
            method: method.to_string(),
            part: part.to_string(),
            status,
            body: body.to_string(),
            echo_headers: false,
            after_auth: false,
        });
    }

    /// As [`fail`](Self::fail), but only for requests that got past
    /// authentication: a refusal of what a credential may do, rather than
    /// of who sent it.
    pub fn fail_authorized(&self, method: &str, part: &str, status: u16, body: &str) {
        self.state.lock().unwrap().failures.push(Failure {
            method: method.to_string(),
            part: part.to_string(),
            status,
            body: body.to_string(),
            echo_headers: false,
            after_auth: true,
        });
    }

    /// As [`fail`](Self::fail), with every request header written into the
    /// body.
    pub fn fail_echoing_headers(&self, method: &str, part: &str, status: u16) {
        self.state.lock().unwrap().failures.push(Failure {
            method: method.to_string(),
            part: part.to_string(),
            status,
            body: String::new(),
            echo_headers: true,
            after_auth: false,
        });
    }

    /// As [`fail_echoing_headers`](Self::fail_echoing_headers), once the
    /// request has passed authentication — so its credentials are among
    /// the headers echoed.
    pub fn fail_authorized_echoing_headers(&self, method: &str, part: &str, status: u16) {
        self.state.lock().unwrap().failures.push(Failure {
            method: method.to_string(),
            part: part.to_string(),
            status,
            body: String::new(),
            echo_headers: true,
            after_auth: true,
        });
    }

    /// Store `bytes` as a blob; returns its digest.
    pub fn put_blob(&self, bytes: &[u8]) -> String {
        let d = digest(bytes);
        self.state
            .lock()
            .unwrap()
            .blobs
            .insert(d.clone(), bytes.to_vec());
        d
    }

    /// The blob `digest`, if the registry holds it.
    pub fn blob(&self, digest: &str) -> Option<Vec<u8>> {
        self.state.lock().unwrap().blobs.get(digest).cloned()
    }

    /// Store `bytes` as the manifest tagged `reference` in `repository`,
    /// served with `media_type`; returns its digest.
    pub fn put_manifest(
        &self,
        repository: &str,
        reference: &str,
        bytes: &[u8],
        media_type: &str,
    ) -> String {
        let d = digest(bytes);
        let mut state = self.state.lock().unwrap();
        state.manifests.insert(
            (repository.to_string(), reference.to_string()),
            bytes.to_vec(),
        );
        state
            .manifests
            .insert((repository.to_string(), d.clone()), bytes.to_vec());
        if media_type != MANIFEST_TYPE {
            state
                .manifest_types
                .insert(d.clone(), media_type.to_string());
        }
        d
    }

    /// Claim `claimed` as the digest of the manifest tagged `tag`, and serve
    /// the real manifest's bytes under that digest too: a registry whose
    /// content does not match what it says it is.
    pub fn lie_about_manifest(&self, repository: &str, tag: &str, claimed: &str) {
        let mut state = self.state.lock().unwrap();
        let body = state
            .manifests
            .get(&(repository.to_string(), tag.to_string()))
            .cloned()
            .expect("a manifest to lie about");
        state
            .manifests
            .insert((repository.to_string(), claimed.to_string()), body);
        state.digest_lies.insert(
            (repository.to_string(), tag.to_string()),
            claimed.to_string(),
        );
    }

    /// Copy every manifest and blob `other` holds into this registry.
    pub fn mirror_from(&self, other: &FakeRegistry) {
        let theirs = other.state.lock().unwrap();
        let mut ours = self.state.lock().unwrap();
        for (key, value) in &theirs.manifests {
            ours.manifests.insert(key.clone(), value.clone());
        }
        for (key, value) in &theirs.blobs {
            ours.blobs.insert(key.clone(), value.clone());
        }
    }

    /// Every request so far, as `METHOD path`.
    pub fn log(&self) -> Vec<String> {
        self.state.lock().unwrap().log.clone()
    }

    /// Every request so far, with its `Authorization` header.
    pub fn seen(&self) -> Vec<Seen> {
        self.state.lock().unwrap().seen.clone()
    }

    pub fn clear_log(&self) {
        let mut state = self.state.lock().unwrap();
        state.log.clear();
        state.seen.clear();
    }

    /// How many logged requests had `method` and a path containing `part`.
    pub fn count(&self, method: &str, part: &str) -> usize {
        self.log()
            .iter()
            .filter(|line| line.starts_with(&format!("{} ", method)) && line.contains(part))
            .count()
    }

    /// The manifest tagged `tag` in `repository`, if there is one.
    pub fn manifest(&self, repository: &str, tag: &str) -> Option<serde_json::Value> {
        let state = self.state.lock().unwrap();
        let bytes = state
            .manifests
            .get(&(repository.to_string(), tag.to_string()))?;
        serde_json::from_slice(bytes).ok()
    }

    pub fn tags(&self, repository: &str) -> Vec<String> {
        let state = self.state.lock().unwrap();
        let mut tags: Vec<String> = state
            .manifests
            .keys()
            .filter(|(repo, reference)| repo == repository && !reference.starts_with("sha256:"))
            .map(|(_, tag)| tag.clone())
            .collect();
        tags.sort();
        tags
    }
}

pub fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

struct Request {
    method: String,
    path: String,
    query_text: String,
    query: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn read_request(stream: &TcpStream) -> std::io::Result<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_string());
        }
    }
    let length: usize = headers
        .get("content-length")
        .and_then(|l| l.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let (path, query_text) = target.split_once('?').unwrap_or((&target, ""));
    let query = url::form_urlencoded::parse(query_text.as_bytes())
        .into_owned()
        .collect();
    Ok(Request {
        method,
        path: path.to_string(),
        query_text: query_text.to_string(),
        query,
        headers,
        body,
    })
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// Send only this many bytes of the body, then close.
    cut_at: Option<usize>,
}

impl Reply {
    fn status(status: u16) -> Reply {
        Reply {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            cut_at: None,
        }
    }

    fn header(mut self, name: &str, value: &str) -> Reply {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    fn body(mut self, body: Vec<u8>) -> Reply {
        self.body = body;
        self
    }
}

fn serve(mut stream: TcpStream, state: &Mutex<State>, port: u16) -> std::io::Result<()> {
    let request = read_request(&stream)?;
    let reply = handle(&request, state, port);
    let reason = match reply.status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        307 => "Temporary Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        _ => "Status",
    };
    let mut head = format!("HTTP/1.1 {} {}\r\n", reply.status, reason);
    for (name, value) in &reply.headers {
        head.push_str(&format!("{}: {}\r\n", name, value));
    }
    if !reply.headers.iter().any(|(n, _)| n == "Content-Length") {
        head.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
    }
    head.push_str("Connection: close\r\n\r\n");
    stream.write_all(head.as_bytes())?;
    if request.method != "HEAD" {
        let end = reply.cut_at.unwrap_or(reply.body.len());
        stream.write_all(&reply.body[..end])?;
    }
    stream.flush()
}

fn handle(request: &Request, state: &Mutex<State>, port: u16) -> Reply {
    let mut state = state.lock().unwrap();
    let auth_header = request.headers.get("authorization").cloned();
    state.log.push(format!(
        "{} {}{}",
        request.method,
        request.path,
        if auth_header.is_some() { " [auth]" } else { "" }
    ));
    state.seen.push(Seen {
        method: request.method.clone(),
        path: request.path.clone(),
        query: request.query_text.clone(),
        authorization: auth_header.clone(),
    });

    if let Some(reply) = failure(&state, request, false) {
        return reply;
    }

    if request.path == "/token" {
        let Some(auth) = state.auth.clone() else {
            return Reply::status(404);
        };
        let field = state
            .token_field
            .clone()
            .unwrap_or_else(|| "token".to_string());
        let issue = || {
            Reply::status(200)
                .header("Content-Type", "application/json")
                .body(format!("{{\"{}\":\"{}\"}}", field, TOKEN).into_bytes())
        };
        if request.method == "POST" {
            let form: HashMap<String, String> = url::form_urlencoded::parse(&request.body)
                .into_owned()
                .collect();
            let granted = form.get("grant_type").map(String::as_str) == Some("refresh_token")
                && form
                    .get("refresh_token")
                    .is_some_and(|t| state.refresh_tokens.contains(t));
            return if granted { issue() } else { Reply::status(401) };
        }
        if auth_header.as_deref() == Some(basic(&auth).as_str()) {
            return issue();
        }
        return Reply::status(401);
    }

    let Some(rest) = request.path.strip_prefix("/v2/") else {
        return Reply::status(404);
    };
    if let Some(status) = state.refuse {
        return Reply::status(status);
    }
    if let Some(auth) = &state.auth {
        if state.basic {
            if auth_header.as_deref() != Some(basic(auth).as_str()) {
                return Reply::status(401).header("WWW-Authenticate", "Basic realm=\"fake\"");
            }
        } else if auth_header.as_deref() != Some(format!("Bearer {}", TOKEN).as_str()) {
            let realm = state
                .realm
                .clone()
                .unwrap_or_else(|| format!("http://{}:{}/token", auth.realm_host, port));
            let challenge = format!("Bearer realm=\"{}\",service=\"fake\"", realm);
            return Reply::status(401).header("WWW-Authenticate", &challenge);
        }
    }
    if let Some(reply) = failure(&state, request, true) {
        return reply;
    }
    if rest.is_empty() {
        return Reply::status(200);
    }

    if let Some(repository) = rest.strip_suffix("/tags/list") {
        // Two tags a page, whatever the client asks for: real registries cap
        // page sizes, and a client has to follow the Link header to see more.
        let mut tags: Vec<String> = state
            .manifests
            .keys()
            .filter(|(repo, reference)| repo == repository && !reference.starts_with("sha256:"))
            .map(|(_, tag)| tag.clone())
            .collect();
        if tags.is_empty() && !state.endless_tags {
            return Reply::status(404);
        }
        tags.sort();
        let after = request.query.get("last").cloned().unwrap_or_default();
        let page: Vec<String> = tags
            .iter()
            .filter(|t| t.as_str() > after.as_str())
            .take(2)
            .cloned()
            .collect();
        let mut reply = Reply::status(200).header("Content-Type", "application/json");
        let base = state.tags_next_base.clone().unwrap_or_default();
        if state.endless_tags {
            reply = reply.header(
                "Link",
                &format!(
                    "<{}/v2/{}/tags/list?n=2&last={}>; rel=\"next\"",
                    base, repository, after
                ),
            );
        } else if let Some(last) = page.last() {
            if tags.iter().any(|t| t > last) {
                reply = reply.header(
                    "Link",
                    &format!(
                        "<{}/v2/{}/tags/list?n=2&last={}>; rel=\"next\"",
                        base, repository, last
                    ),
                );
            }
        }
        let body = serde_json::json!({ "name": repository, "tags": page });
        return reply.body(body.to_string().into_bytes());
    }

    if let Some((repository, reference)) = rest.split_once("/manifests/") {
        let key = (repository.to_string(), reference.to_string());
        return match request.method.as_str() {
            "PUT" => {
                let body = request.body.clone();
                let d = digest(&body);
                state.manifests.insert(key, body.clone());
                state
                    .manifests
                    .insert((repository.to_string(), d.clone()), body);
                Reply::status(201).header("Docker-Content-Digest", &d)
            }
            "GET" | "HEAD" => match state.manifests.get(&key) {
                Some(body) => {
                    let claimed = state
                        .digest_lies
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| digest(body));
                    let media_type = state
                        .manifest_types
                        .get(&digest(body))
                        .cloned()
                        .unwrap_or_else(|| MANIFEST_TYPE.to_string());
                    let mut reply = Reply::status(200)
                        .header("Content-Type", &media_type)
                        .header("Content-Length", &body.len().to_string());
                    if !state.omit_manifest_digest {
                        reply = reply.header("Docker-Content-Digest", &claimed);
                    }
                    reply.body(body.clone())
                }
                None => Reply::status(404),
            },
            _ => Reply::status(400),
        };
    }

    if let Some((repository, upload)) = rest.split_once("/blobs/uploads/") {
        return match request.method.as_str() {
            "POST" => {
                state.next_upload += 1;
                let id = format!("u{}", state.next_upload);
                state.uploads.insert(id.clone(), repository.to_string());
                // A relative location with a query of its own: the client must
                // resolve it and add the digest to what is there.
                let location = format!(
                    "{}/v2/{}/blobs/uploads/{}?state=opaque",
                    state.upload_base.clone().unwrap_or_default(),
                    repository,
                    id
                );
                Reply::status(202).header("Location", &location)
            }
            "PUT" => {
                let started = state.uploads.remove(upload).is_some() || state.accept_any_upload;
                if !started || request.query.get("state").map(String::as_str) != Some("opaque") {
                    return Reply::status(404);
                }
                let wanted = request.query.get("digest").cloned().unwrap_or_default();
                if digest(&request.body) != wanted {
                    return Reply::status(400);
                }
                state.blobs.insert(wanted.clone(), request.body.clone());
                Reply::status(201).header("Docker-Content-Digest", &wanted)
            }
            _ => Reply::status(400),
        };
    }

    if let Some((_, wanted)) = rest.split_once("/blobs/") {
        if request.method == "GET" {
            if let Some(base) = &state.blob_redirect {
                return Reply::status(307).header("Location", &format!("{}{}", base, request.path));
            }
        }
        let corrupt = state.corrupt || state.corrupt_digests.contains(wanted);
        let truncate = state.truncate_digests.contains(wanted);
        return match state.blobs.get(wanted) {
            Some(body) => {
                let mut body = body.clone();
                if corrupt {
                    body.reverse();
                    body.push(b'!');
                }
                let mut reply = Reply::status(200)
                    .header("Content-Length", &body.len().to_string())
                    .body(body.clone());
                if truncate {
                    reply.cut_at = Some(body.len() / 2);
                }
                reply
            }
            None => Reply::status(404),
        };
    }
    Reply::status(404)
}

fn basic(auth: &Auth) -> String {
    format!(
        "Basic {}",
        base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{}:{}", auth.user, auth.password)
        )
    )
}

/// The failure set for `request`, if any: those for any request, or with
/// `after_auth` those for one that passed authentication.
fn failure(state: &State, request: &Request, after_auth: bool) -> Option<Reply> {
    let failure = state.failures.iter().find(|f| {
        f.after_auth == after_auth && f.method == request.method && request.path.contains(&f.part)
    })?;
    let body = if failure.echo_headers {
        let mut lines: Vec<String> = request
            .headers
            .iter()
            .map(|(name, value)| format!("{}: {}", name, value))
            .collect();
        lines.sort();
        lines.join("\n")
    } else {
        failure.body.clone()
    };
    Some(Reply::status(failure.status).body(body.into_bytes()))
}

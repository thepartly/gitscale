//! A fake OCI registry for the tests: the slice of the Distribution API
//! gitscale calls, served from memory on 127.0.0.1, with a request log and
//! switches for the failures a real registry can produce.
#![allow(dead_code)]

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

const MANIFEST_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
/// The token the fake token service hands out for good credentials.
const TOKEN: &str = "fake-registry-token";

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

#[derive(Default)]
struct State {
    /// (repository, tag or digest) -> manifest bytes.
    manifests: HashMap<(String, String), Vec<u8>>,
    blobs: HashMap<String, Vec<u8>>,
    uploads: HashMap<String, String>,
    next_upload: u64,
    log: Vec<String>,
    auth: Option<Auth>,
    /// Answer every `/v2/` request with this status.
    refuse: Option<u16>,
    /// Serve blobs with their bytes changed.
    corrupt: bool,
}

pub struct FakeRegistry {
    pub addr: String,
    state: Arc<Mutex<State>>,
}

impl FakeRegistry {
    pub fn start() -> FakeRegistry {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind the fake registry");
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

    /// Every request so far, as `METHOD path`.
    pub fn log(&self) -> Vec<String> {
        self.state.lock().unwrap().log.clone()
    }

    pub fn clear_log(&self) {
        self.state.lock().unwrap().log.clear();
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

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

struct Request {
    method: String,
    path: String,
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
        query,
        headers,
        body,
    })
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn status(status: u16) -> Reply {
        Reply {
            status,
            headers: Vec::new(),
            body: Vec::new(),
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
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
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
        stream.write_all(&reply.body)?;
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

    if request.path == "/token" {
        let Some(auth) = state.auth.clone() else {
            return Reply::status(404);
        };
        let expected = format!(
            "Basic {}",
            base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                format!("{}:{}", auth.user, auth.password)
            )
        );
        if auth_header.as_deref() == Some(expected.as_str()) {
            return Reply::status(200)
                .header("Content-Type", "application/json")
                .body(format!("{{\"token\":\"{}\"}}", TOKEN).into_bytes());
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
        if auth_header.as_deref() != Some(format!("Bearer {}", TOKEN).as_str()) {
            let challenge = format!(
                "Bearer realm=\"http://{}:{}/token\",service=\"fake\"",
                auth.realm_host, port
            );
            return Reply::status(401).header("WWW-Authenticate", &challenge);
        }
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
        if tags.is_empty() {
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
        if let Some(last) = page.last() {
            if tags.iter().any(|t| t > last) {
                reply = reply.header(
                    "Link",
                    &format!(
                        "</v2/{}/tags/list?n=2&last={}>; rel=\"next\"",
                        repository, last
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
                Some(body) => Reply::status(200)
                    .header("Content-Type", MANIFEST_TYPE)
                    .header("Docker-Content-Digest", &digest(body))
                    .header("Content-Length", &body.len().to_string())
                    .body(body.clone()),
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
                let location = format!("/v2/{}/blobs/uploads/{}?state=opaque", repository, id);
                Reply::status(202).header("Location", &location)
            }
            "PUT" => {
                if state.uploads.remove(upload).is_none()
                    || request.query.get("state").map(String::as_str) != Some("opaque")
                {
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
        let corrupt = state.corrupt;
        return match state.blobs.get(wanted) {
            Some(body) => {
                let mut body = body.clone();
                if corrupt {
                    body.reverse();
                    body.push(b'!');
                }
                Reply::status(200)
                    .header("Content-Length", &body.len().to_string())
                    .body(body)
            }
            None => Reply::status(404),
        };
    }
    Reply::status(404)
}

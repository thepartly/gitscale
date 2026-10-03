//! A git smart-HTTP server on 127.0.0.1, for the CI authentication tests.
//!
//! It serves the bare repositories under one directory through
//! `git http-backend`, demands Basic credentials on every request — as a
//! private forge does — and logs the host each request was addressed to and the
//! credentials it carried. One listener answers for every name of the loopback
//! address, so `localhost:<port>` and `127.0.0.1:<port>` are two hosts to git
//! and one server here: the log tells which one a token was sent to.
//!
//! The server stops when the value is dropped.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use base64::Engine;

/// One request as the server saw it.
#[derive(Clone, Debug)]
pub struct Request {
    /// The `Host` header without the port: the name git addressed.
    pub host: String,
    pub path: String,
    /// The Basic credentials, decoded to `user:password`.
    pub credentials: Option<String>,
}

pub struct GitHttp {
    pub port: u16,
    log: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl GitHttp {
    /// Serve the bare repositories under `root`. A request that carries
    /// credentials and names a path containing `forbidden` is answered 403 —
    /// a token the forge accepts but that may not read that project.
    pub fn start(root: &Path) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let root = root.to_path_buf();
        let accept = {
            let (log, stop) = (log.clone(), stop.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let (root, log) = (root.clone(), log.clone());
                    std::thread::spawn(move || {
                        let _ = serve(stream, &root, &log);
                    });
                }
            })
        };
        Self {
            port,
            log,
            stop,
            accept: Some(accept),
        }
    }

    /// `http://<host>:<port>/<path>`.
    pub fn url(&self, host: &str, path: &str) -> String {
        format!("http://{}:{}/{}", host, self.port, path)
    }

    /// Every request so far.
    pub fn requests(&self) -> Vec<Request> {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for GitHttp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.accept.take() {
            let _ = handle.join();
        }
    }
}

fn serve(stream: TcpStream, root: &Path, log: &Mutex<Vec<Request>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(());
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();

    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.push((name.trim().to_lowercase(), value.trim().to_string()));
        }
    }
    let header = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    };

    let body = if header("transfer-encoding").is_some_and(|v| v.contains("chunked")) {
        read_chunked(&mut reader)?
    } else {
        let len: usize = header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0; len];
        reader.read_exact(&mut body)?;
        body
    };

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };
    let host = header("host")
        .unwrap_or_default()
        .rsplit_once(':')
        .map(|(h, _)| h.to_string())
        .unwrap_or_default();
    let credentials = header("authorization")
        .and_then(|v| v.strip_prefix("Basic ").map(str::to_string))
        .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
        .map(|raw| String::from_utf8_lossy(&raw).into_owned());
    log.lock().unwrap().push(Request {
        host,
        path: path.clone(),
        credentials: credentials.clone(),
    });

    let mut out = stream;
    if credentials.is_none() {
        let msg = b"credentials required\n";
        write!(
            out,
            "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"test\"\r\n\
             Content-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            msg.len()
        )?;
        out.write_all(msg)?;
        return out.flush();
    }
    if path.contains("forbidden") {
        let msg = b"forbidden\n";
        write!(
            out,
            "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n",
            msg.len()
        )?;
        out.write_all(msg)?;
        return out.flush();
    }

    let mut cgi = Command::new("git");
    cgi.arg("http-backend")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", root)
        .env("REQUEST_METHOD", &method)
        .env("PATH_INFO", &path)
        .env("QUERY_STRING", &query)
        .env("CONTENT_LENGTH", body.len().to_string())
        .env("REMOTE_USER", "test")
        .env("REMOTE_ADDR", "127.0.0.1");
    if let Some(ct) = header("content-type") {
        cgi.env("CONTENT_TYPE", ct);
    }
    for (name, value) in &headers {
        let var = format!("HTTP_{}", name.to_uppercase().replace('-', "_"));
        cgi.env(var, value);
    }
    let mut child = cgi
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    child.stdin.take().unwrap().write_all(&body)?;
    let output = child.wait_with_output()?;
    let raw = output.stdout;

    // CGI output: headers, a blank line, the body.
    let split = find(&raw, b"\r\n\r\n")
        .map(|i| (i, 4))
        .or_else(|| find(&raw, b"\n\n").map(|i| (i, 2)));
    let (head, payload) = match split {
        Some((i, n)) => (
            String::from_utf8_lossy(&raw[..i]).into_owned(),
            &raw[i + n..],
        ),
        None => (String::new(), &raw[..]),
    };
    let mut status = "200 OK".to_string();
    let mut reply_headers = String::new();
    for h in head.lines() {
        if let Some(s) = h.strip_prefix("Status:") {
            status = s.trim().to_string();
        } else if !h.trim().is_empty() {
            reply_headers.push_str(h.trim_end());
            reply_headers.push_str("\r\n");
        }
    }
    write!(
        out,
        "HTTP/1.1 {}\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n",
        status,
        reply_headers,
        payload.len()
    )?;
    out.write_all(payload)?;
    out.flush()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn read_chunked(reader: &mut impl BufRead) -> std::io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let mut size = String::new();
        reader.read_line(&mut size)?;
        let size =
            usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16).unwrap_or(0);
        if size == 0 {
            let mut end = String::new();
            reader.read_line(&mut end)?;
            return Ok(body);
        }
        let mut chunk = vec![0; size];
        reader.read_exact(&mut chunk)?;
        body.extend_from_slice(&chunk);
        let mut crlf = String::new();
        reader.read_line(&mut crlf)?;
    }
}

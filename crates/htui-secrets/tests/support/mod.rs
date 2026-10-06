//! A loopback stand-in for Infisical (MOD-10 M2): HTTP/1.1 on a std thread, scripted answers per
//! `(method, path)`, every request recorded **before** it is answered.
//!
//! Extends `htui-store`'s `model.rs` stub with methods, query, headers, body and per-route
//! sequences. No new dev-dependency: `url` and `serde_json` are regular dependencies of the crate.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The scripted routes: `(method, path)` → the answers still to give.
type Script = Arc<Mutex<HashMap<(String, String), VecDeque<Reply>>>>;

/// One scripted answer.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The `Content-Length` announced instead of `body.len()` (see [`Reply::truncated`]).
    pub declared_length: Option<usize>,
    /// How long to wait, after recording the request, before answering (see [`Reply::delayed`]).
    pub delay: Option<Duration>,
}

impl Reply {
    /// `status` with a JSON body and `Content-Type: application/json`.
    pub fn json(status: u16, body: &serde_json::Value) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
            body: serde_json::to_vec(body).expect("a JSON value serialises"),
            declared_length: None,
            delay: None,
        }
    }

    /// `status` with a raw text body (a non-JSON answer).
    pub fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".to_owned(), "text/html".to_owned())],
            body: body.as_bytes().to_vec(),
            declared_length: None,
            delay: None,
        }
    }

    /// `status` with `Location: location` and no body.
    pub fn redirect(status: u16, location: &str) -> Self {
        Self {
            status,
            headers: vec![("Location".to_owned(), location.to_owned())],
            body: Vec::new(),
            declared_length: None,
            delay: None,
        }
    }

    /// One extra header (e.g. `Retry-After`).
    #[must_use]
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// Answers only after `delay`: the request is recorded at once, so a client that gives up
    /// earlier has still been seen. The stub serves one connection at a time, so later
    /// connections wait behind it.
    #[must_use]
    pub fn delayed(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    /// Announces a `Content-Length` of `declared` but sends only the body, then closes: the
    /// client sees the status and headers, and its body read fails part-way.
    #[must_use]
    pub fn truncated(mut self, declared: usize) -> Self {
        assert!(
            declared > self.body.len(),
            "a truncated reply must announce more than it sends"
        );
        self.declared_length = Some(declared);
        self
    }
}

/// One recorded request.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    /// The path without the query.
    pub path: String,
    /// Decoded query pairs, in order.
    pub query: Vec<(String, String)>,
    /// Header names lowercased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// A header by case-insensitive name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Every value of a query key.
    pub fn query(&self, key: &str) -> Vec<&str> {
        self.query
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// The body as JSON. Panics (naming the path, never the body) if it is not JSON.
    pub fn json(&self) -> serde_json::Value {
        match serde_json::from_slice(&self.body) {
            Ok(value) => value,
            Err(_) => panic!("the body sent to {} is not JSON", self.path),
        }
    }
}

/// The stub. Each test owns one.
#[derive(Debug)]
pub struct Stub {
    addr: SocketAddr,
    script: Script,
    seen: Arc<Mutex<Vec<Request>>>,
}

impl Stub {
    /// Binds `127.0.0.1:0` (never `localhost`, which may resolve to `::1` first) and serves
    /// connections one at a time on a std thread.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("the stub's address");
        let script: Script = Arc::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (routes, record) = (Arc::clone(&script), Arc::clone(&seen));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                serve(&stream, &routes, &record);
            }
        });
        Self { addr, script, seen }
    }

    /// Appends `reply` to the route's queue. Each request pops the front; the **last** reply of a
    /// queue is sticky (served for every later request).
    pub fn on(&self, method: &str, path: &str, reply: Reply) -> &Self {
        self.script
            .lock()
            .expect("the stub's script")
            .entry((method.to_owned(), path.to_owned()))
            .or_default()
            .push_back(reply);
        self
    }

    /// `http://127.0.0.1:<port>`.
    pub fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Every request so far, in arrival order.
    pub fn requests(&self) -> Vec<Request> {
        self.seen.lock().expect("the stub's log").clone()
    }

    /// How many requests hit `(method, path)`.
    pub fn count(&self, method: &str, path: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.method == method && r.path == path)
            .count()
    }
}

/// A loopback base URL whose connect is refused at once, for the `Unreachable` cases. Keep it
/// alive for as long as the URL is used.
///
/// Race-free: the port is **held** on `127.0.0.1` for the guard's lifetime, so no stub (they all
/// bind `127.0.0.1:0`) can be handed it, and no wildcard listener can take it either. The URL
/// names `127.0.0.2`, another loopback address (Linux routes all of `127.0.0.0/8` to `lo`) on
/// which nothing listens on that port: the kernel answers the SYN with a reset, a connect error.
/// Pointing at the held listener itself would not do: an unaccepted connection still completes
/// its handshake through the backlog, and the request would time out instead of being refused.
#[derive(Debug)]
pub struct ClosedPort {
    _held: TcpListener,
    base: String,
}

impl ClosedPort {
    /// Reserves the port.
    pub fn new() -> Self {
        let held = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let port = held.local_addr().expect("the port's address").port();
        Self {
            _held: held,
            base: format!("http://127.0.0.2:{port}"),
        }
    }

    /// `http://127.0.0.2:<port>`.
    pub fn base(&self) -> &str {
        &self.base
    }
}

/// One request on one connection, recorded, then answered with `Connection: close`.
fn serve(stream: &TcpStream, script: &Script, seen: &Mutex<Vec<Request>>) {
    // H-7: headers and body come from the same reader, which may already hold body bytes.
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return;
    };
    let (method, target) = (method.to_owned(), target.to_owned());
    let mut headers = Vec::new();
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let (name, value) = (name.trim().to_ascii_lowercase(), value.trim().to_owned());
            if name == "content-length" {
                length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }
    let mut body = vec![0; length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (
            path.to_owned(),
            url::form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect(),
        ),
        None => (target.clone(), Vec::new()),
    };
    let key = (method.clone(), path.clone());
    seen.lock().expect("the stub's log").push(Request {
        method,
        path,
        query,
        headers,
        body,
    });
    let reply = {
        let mut script = script.lock().expect("the stub's script");
        match script.get_mut(&key) {
            Some(queue) if queue.len() > 1 => queue.pop_front(),
            Some(queue) => queue.front().cloned(),
            None => None,
        }
    }
    .unwrap_or_else(|| {
        Reply::json(
            418,
            &serde_json::json!({"message": "stub: unscripted route"}),
        )
    });
    if let Some(delay) = reply.delay {
        std::thread::sleep(delay);
    }
    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.declared_length.unwrap_or(reply.body.len())
    );
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let mut out = stream;
    let _ = out.write_all(head.as_bytes());
    let _ = out.write_all(&reply.body);
    let _ = out.flush();
}

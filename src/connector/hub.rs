//! The hub the Oido Browser Connector talks to, served on this computer: the same two endpoints
//! oido-core serves in the cloud, so the extension works unchanged with its base URL set to
//! `http://127.0.0.1:PORT`.
//!
//! - `GET /api/browser/stream` (down): a server-sent event stream; each command is one
//!   `data: {"id", "action", "params"}` frame. A newer stream replaces the one before.
//! - `POST /api/browser/result` (up): the extension's answer to a command, `{"id", "data", "error"}`.
//!
//! Both need `Authorization: Bearer <token>`, the token shown on the Settings screen. Only this
//! computer can reach the port.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The port the hub listens on, on 127.0.0.1. Fixed, so the extension's base URL stays right.
pub const PORT: u16 = 47821;

/// How long one command waits for the extension's answer.
const ANSWER_WAIT: Duration = Duration::from_secs(60);
/// How long a command waits for an extension that was connected a moment ago to come back.
const RECONNECT_WAIT: Duration = Duration::from_secs(10);
/// How recently the extension must have been connected for a command to wait for it.
const RECENT: Duration = Duration::from_secs(60);
/// How often an idle stream sends a comment, so a dead socket is noticed.
const KEEPALIVE: Duration = Duration::from_secs(15);
/// The largest request the hub reads, in bytes: a page read is well under this.
const BODY_LIMIT: usize = 8 * 1024 * 1024;

/// The extension's answer to a command.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub data: Value,
    pub error: String,
}

/// Why a command got no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// No extension is connected.
    NotConnected,
    /// The command went out, and the extension went away before answering.
    Disconnected,
    /// The command went out, and no answer came in time.
    TimedOut,
}

struct Stream {
    generation: u64,
    frames: Sender<Vec<u8>>,
}

#[derive(Default)]
struct State {
    stream: Option<Stream>,
    /// Commands waiting for their answer, by id: the stream they went out on, and where to answer.
    waiters: HashMap<String, (u64, Sender<Option<Reply>>)>,
    last_connected: Option<Instant>,
}

/// The hub's shared state: the token, the extension's stream, the commands waiting.
#[derive(Default)]
pub struct Hub {
    token: Mutex<String>,
    state: Mutex<State>,
    generations: AtomicU64,
    /// Called when the extension connects or goes away, so the screen can say so.
    on_change: Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hub")
            .field("connected", &self.connected())
            .finish_non_exhaustive()
    }
}

impl Hub {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sets the token the extension must send. A stream opened with the old one keeps running
    /// until the extension reconnects.
    pub fn set_token(&self, token: &str) {
        *self
            .token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = token.to_owned();
    }

    pub fn set_on_change(&self, f: impl Fn() + Send + 'static) {
        *self
            .on_change
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Box::new(f));
    }

    fn changed(&self) {
        if let Some(f) = self
            .on_change
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            f();
        }
    }

    pub fn connected(&self) -> bool {
        self.state().stream.is_some()
    }

    fn authorized(&self, header: Option<&str>) -> bool {
        let token = self
            .token
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let given = header
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(str::trim)
            .unwrap_or_default();
        !token.is_empty() && same(given.as_bytes(), token.as_bytes())
    }

    /// Sends `action` with `params` to the extension and waits for its answer.
    ///
    /// # Errors
    ///
    /// No extension connected, or it went away or said nothing in time.
    pub fn dispatch(&self, action: &str, params: Value) -> Result<Reply, Failure> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let frame = format!(
            "data: {}\n\n",
            json!({"id": id, "action": action, "params": params})
        );
        let (answer, wait) = mpsc::channel();
        let deadline = Instant::now() + RECONNECT_WAIT;
        loop {
            {
                let mut state = self.state();
                if let Some(stream) = &state.stream {
                    let generation = stream.generation;
                    if stream.frames.send(frame.clone().into_bytes()).is_ok() {
                        state.waiters.insert(id.clone(), (generation, answer));
                        break;
                    }
                }
                let recent = state.last_connected.is_some_and(|t| t.elapsed() < RECENT);
                if !recent || Instant::now() >= deadline {
                    return Err(Failure::NotConnected);
                }
            }
            // Just dropped: the extension reconnects within seconds.
            std::thread::sleep(Duration::from_millis(200));
        }
        let result = match wait.recv_timeout(ANSWER_WAIT) {
            Ok(Some(reply)) => Ok(reply),
            Ok(None) | Err(RecvTimeoutError::Disconnected) => Err(Failure::Disconnected),
            Err(RecvTimeoutError::Timeout) => Err(Failure::TimedOut),
        };
        self.state().waiters.remove(&id);
        result
    }

    /// The extension's answer to command `id`. One nobody waits for (a repeat) is dropped.
    fn resolve(&self, id: &str, reply: Reply) {
        if let Some((_, answer)) = self.state().waiters.remove(id) {
            let _ = answer.send(Some(reply));
        }
    }

    /// A new stream: it replaces any before it. Returns its generation and where its frames come.
    fn open_stream(&self) -> (u64, mpsc::Receiver<Vec<u8>>) {
        let generation = self.generations.fetch_add(1, Ordering::Relaxed) + 1;
        let (frames, rx) = mpsc::channel();
        {
            let mut state = self.state();
            state.stream = Some(Stream { generation, frames });
            state.last_connected = Some(Instant::now());
        }
        self.changed();
        (generation, rx)
    }

    /// Stream `generation` ended: if it is still the current one, the extension is gone, and the
    /// commands sent on it will not be answered.
    fn close_stream(&self, generation: u64) {
        {
            let mut state = self.state();
            if state
                .stream
                .as_ref()
                .is_none_or(|s| s.generation != generation)
            {
                return;
            }
            state.stream = None;
            state.last_connected = Some(Instant::now());
            let lost: Vec<String> = state
                .waiters
                .iter()
                .filter(|(_, (g, _))| *g == generation)
                .map(|(id, _)| id.clone())
                .collect();
            for id in lost {
                if let Some((_, answer)) = state.waiters.remove(&id) {
                    let _ = answer.send(None);
                }
            }
        }
        self.changed();
    }
}

/// Compares in time that does not depend on where the bytes differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Listens on `127.0.0.1:port` in the background, one thread per connection.
///
/// # Errors
///
/// The port could not be opened (another app holds it), in words.
pub fn serve(hub: Arc<Hub>, port: u16) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("could not listen on 127.0.0.1:{port} ({e})"))?;
    std::thread::Builder::new()
        .name("oido-connector".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let hub = Arc::clone(&hub);
                let _ = std::thread::Builder::new()
                    .name("oido-connector-conn".into())
                    .spawn(move || handle(&hub, stream));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

struct Request {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let path = parts.next()?.to_owned();
    let (mut authorization, mut length) = (None, 0usize);
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 {
            return None;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "authorization" => authorization = Some(value.trim().to_owned()),
                "content-length" => length = value.trim().parse().ok()?,
                _ => {}
            }
        }
    }
    if length > BODY_LIMIT {
        return None;
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Request {
        method,
        path,
        authorization,
        body,
    })
}

fn respond(mut stream: &TcpStream, status: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn handle(hub: &Hub, stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let Some(req) = read_request(&stream) else {
        return respond(&stream, "400 Bad Request", r#"{"error":"bad request"}"#);
    };
    let path = req.path.split('?').next().unwrap_or_default();
    let known = matches!(
        (req.method.as_str(), path),
        ("GET", "/api/browser/stream") | ("POST", "/api/browser/result")
    );
    if !known {
        return respond(&stream, "404 Not Found", r#"{"error":"not found"}"#);
    }
    if !hub.authorized(req.authorization.as_deref()) {
        return respond(&stream, "401 Unauthorized", r#"{"error":"bad token"}"#);
    }
    if req.method == "GET" {
        return run_stream(hub, stream);
    }
    let Ok(v) = serde_json::from_slice::<Value>(&req.body) else {
        return respond(&stream, "400 Bad Request", r#"{"error":"not JSON"}"#);
    };
    let Some(id) = v["id"].as_str().filter(|id| !id.is_empty()) else {
        return respond(
            &stream,
            "400 Bad Request",
            r#"{"error":"missing command id"}"#,
        );
    };
    hub.resolve(
        id,
        Reply {
            data: v.get("data").cloned().unwrap_or(Value::Null),
            error: v["error"].as_str().unwrap_or_default().to_owned(),
        },
    );
    respond(&stream, "200 OK", r#"{"accepted":true}"#);
}

/// Holds the extension's stream open, writing each command as it comes, until it closes or a
/// newer stream replaces it.
fn run_stream(hub: &Hub, mut stream: TcpStream) {
    let opened = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n: connected\n\n",
    );
    if opened.is_err() {
        return;
    }
    let (generation, frames) = hub.open_stream();
    loop {
        let frame = match frames.recv_timeout(KEEPALIVE) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => b": ping\n\n".to_vec(),
            // Replaced by a newer stream.
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if stream
            .write_all(&frame)
            .and_then(|()| stream.flush())
            .is_err()
        {
            break;
        }
    }
    hub.close_stream(generation);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A stand-in for the extension: opens the stream, answers each command with `answer`.
    fn extension(port: u16, token: &str, answer: fn(&Value) -> Value) {
        let token = token.to_owned();
        std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(
                s,
                "GET /api/browser/stream HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\n\r\n"
            )
            .unwrap();
            let mut reader = BufReader::new(s);
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                if let Some(json) = line.trim_end().strip_prefix("data: ") {
                    let cmd: Value = serde_json::from_str(json).unwrap();
                    let body = json!({"id": cmd["id"], "data": answer(&cmd)}).to_string();
                    let mut post = TcpStream::connect(("127.0.0.1", port)).unwrap();
                    write!(
                        post,
                        "POST /api/browser/result HTTP/1.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                    let mut ok = String::new();
                    let _ = post.read_to_string(&mut ok);
                }
                line.clear();
            }
        });
    }

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn post(port: u16, auth: &str, path: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            s,
            "POST {path} HTTP/1.1\r\nAuthorization: {auth}\r\nContent-Length: 2\r\n\r\n{{}}"
        )
        .unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    #[test]
    fn a_command_goes_down_the_stream_and_its_answer_comes_back() {
        let hub = Arc::new(Hub::default());
        hub.set_token("secret");
        let port = free_port();
        serve(Arc::clone(&hub), port).unwrap();
        assert_eq!(
            hub.dispatch("read", json!({})),
            Err(Failure::NotConnected),
            "nothing connected yet"
        );
        extension(port, "secret", |cmd| {
            json!(format!("did {}", cmd["action"]))
        });
        let until = Instant::now() + Duration::from_secs(5);
        while !hub.connected() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(hub.connected());
        let reply = hub.dispatch("click", json!({"selector": "c1"})).unwrap();
        assert_eq!(reply.data, json!("did \"click\""));
    }

    #[test]
    fn a_wrong_token_or_an_unknown_path_is_refused() {
        let hub = Arc::new(Hub::default());
        hub.set_token("secret");
        let port = free_port();
        serve(Arc::clone(&hub), port).unwrap();
        assert!(post(port, "Bearer nope", "/api/browser/result").starts_with("HTTP/1.1 401"));
        assert!(post(port, "Bearer secret", "/elsewhere").starts_with("HTTP/1.1 404"));
        assert!(post(port, "Bearer secret", "/api/browser/result").starts_with("HTTP/1.1 400"));
        let empty = Hub::default();
        assert!(
            !empty.authorized(Some("Bearer ")),
            "no token set, nothing passes"
        );
    }
}

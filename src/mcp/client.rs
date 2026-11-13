//! Talking to one MCP server: JSON-RPC over HTTP for a remote one, over a child process's standard
//! input and output for a local one. Everything here blocks, so call it off the UI thread.
//!
//! A remote server is dealt with afresh each time (initialize, then the call): nothing is kept
//! between calls, so there is nothing to expire. A local server is started on its first use and
//! kept for the next, since starting one (`npx` and the like) can take seconds; it is ended by
//! [`stop`] when the server is removed or turned off, and when it stops answering.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio as Pipe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Secrets, Tool, Transport, oauth};

const PROTOCOL: &str = "2025-06-18";
/// Starting a server, shaking hands and listing its tools.
const LIST_TIMEOUT: Duration = Duration::from_secs(60);
/// A tool call, including the handshake before it.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// A tool's output is cut to this many bytes before the model sees it.
const OUTPUT_LIMIT: usize = 32 * 1024;
/// A server's error output kept to explain why it ended.
const STDERR_KEEP: usize = 1500;
/// A listing that goes on past this many pages is not a listing.
const MAX_PAGES: usize = 10;

/// Everything a worker needs to reach a server, opened on the UI thread (the vault cannot leave it).
pub struct Target {
    pub id: String,
    pub transport: Transport,
    pub secrets: Secrets,
}

/// How a call went, and the sign-in tokens it had to renew on the way (to be saved).
pub struct Outcome<T> {
    pub result: Result<T, String>,
    pub tokens: Option<oauth::Tokens>,
}

enum Failure {
    /// The server wants a sign-in it did not get, or no longer accepts it.
    Unauthorized,
    Other(String),
}

impl Failure {
    fn other(why: impl Into<String>) -> Failure {
        Failure::Other(why.into())
    }
}

/// How long a step may take and how to tell it to stop.
struct Ctl<'a> {
    cancel: &'a AtomicBool,
    timeout: Duration,
}

trait Rpc {
    /// The handshake that must come before anything else.
    fn open(&mut self, ctl: &Ctl) -> Result<(), Failure>;
    fn request(&mut self, method: &str, params: Value, ctl: &Ctl) -> Result<Value, Failure>;
}

/// The tools a server offers.
pub fn list_tools(target: Target, cancel: &AtomicBool) -> Outcome<Vec<Tool>> {
    run(target, cancel, LIST_TIMEOUT, |rpc, ctl| {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let params = cursor
                .take()
                .map_or_else(|| json!({}), |c| json!({"cursor": c}));
            let page = rpc.request("tools/list", params, ctl)?;
            tools.extend(
                page["tools"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(tool_of),
            );
            cursor = page["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    })
}

/// Calls one tool, and gives what it returned in words for the model.
pub fn call_tool(target: Target, tool: &str, args: Value, cancel: &AtomicBool) -> Outcome<String> {
    run(target, cancel, CALL_TIMEOUT, |rpc, ctl| {
        let result = rpc.request("tools/call", json!({"name": tool, "arguments": args}), ctl)?;
        Ok(render(&result))
    })
}

/// Ends a local server, if one is running for `id`. A remote server has nothing to end.
pub fn stop(id: &str) {
    if let Ok(mut pool) = pool().lock() {
        pool.remove(id);
    }
}

/// Ends every local server that is running, when the app quits.
pub fn stop_all() {
    if let Ok(mut pool) = pool().lock() {
        pool.clear();
    }
}

fn tool_of(v: &Value) -> Option<Tool> {
    Some(Tool {
        name: v["name"].as_str()?.to_owned(),
        description: v["description"].as_str().unwrap_or_default().to_owned(),
        schema: v["inputSchema"].clone(),
    })
}

/// A tool's result as text. Pictures and sound cannot be shown to the model here, so they are
/// named, not dropped silently.
fn render(result: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in result["content"].as_array().into_iter().flatten() {
        let mime = c["mimeType"].as_str().unwrap_or("unknown type");
        match c["type"].as_str() {
            Some("text") => parts.push(c["text"].as_str().unwrap_or_default().to_owned()),
            Some(kind @ ("image" | "audio")) => {
                parts.push(format!(
                    "[{kind} ({mime}) returned; it cannot be shown here]"
                ));
            }
            Some("resource") => {
                let r = &c["resource"];
                parts.push(
                    r["text"]
                        .as_str()
                        .or_else(|| r["uri"].as_str())
                        .unwrap_or_default()
                        .to_owned(),
                );
            }
            Some("resource_link") => {
                parts.push(c["uri"].as_str().unwrap_or_default().to_owned());
            }
            _ => {}
        }
    }
    if parts.is_empty()
        && let Some(structured) = result.get("structuredContent")
    {
        parts.push(structured.to_string());
    }
    let mut text = parts.join("\n");
    if text.trim().is_empty() {
        text = "(no output)".to_owned();
    }
    if text.len() > OUTPUT_LIMIT {
        let mut end = OUTPUT_LIMIT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[output cut]");
    }
    if result["isError"].as_bool() == Some(true) {
        format!("Error: {text}")
    } else {
        text
    }
}

/// Runs `op` against the server, renewing a sign-in that has run out, once, if that is what went
/// wrong.
fn run<T>(
    mut target: Target,
    cancel: &AtomicBool,
    timeout: Duration,
    op: impl Fn(&mut dyn Rpc, &Ctl) -> Result<T, Failure>,
) -> Outcome<T> {
    let ctl = Ctl { cancel, timeout };
    let mut renewed = None;
    let mut force = false;
    for attempt in 0..2 {
        if let Some(old) = target.secrets.oauth.clone()
            && (force || old.expiring())
            && old.refresh_token.is_some()
        {
            // If it cannot be renewed the old token may still be good; if not, the server's 401
            // says so below.
            if let Ok(fresh) = oauth::refresh(&old) {
                target.secrets.oauth = Some(fresh.clone());
                renewed = Some(fresh);
            }
        }
        let result = reach(&target, &ctl, &op);
        let can_renew = attempt == 0
            && target
                .secrets
                .oauth
                .as_ref()
                .is_some_and(|t| t.refresh_token.is_some());
        match result {
            Err(Failure::Unauthorized) if can_renew => force = true,
            Err(Failure::Unauthorized) => {
                return Outcome {
                    result: Err("the server did not accept the sign-in or key. Sign in or add the key again on the MCP page.".to_owned()),
                    tokens: renewed,
                };
            }
            Err(Failure::Other(why)) => {
                if matches!(target.transport, Transport::Stdio { .. }) {
                    // A server that failed may be wedged: the next try starts it afresh.
                    stop(&target.id);
                }
                return Outcome {
                    result: Err(why),
                    tokens: renewed,
                };
            }
            Ok(v) => {
                return Outcome {
                    result: Ok(v),
                    tokens: renewed,
                };
            }
        }
    }
    Outcome {
        result: Err("the server did not accept the sign-in.".to_owned()),
        tokens: renewed,
    }
}

fn reach<T>(
    target: &Target,
    ctl: &Ctl,
    op: &impl Fn(&mut dyn Rpc, &Ctl) -> Result<T, Failure>,
) -> Result<T, Failure> {
    match &target.transport {
        Transport::Http { url } => {
            let bearer = target
                .secrets
                .oauth
                .as_ref()
                .map(|t| t.access_token.clone())
                .or_else(|| target.secrets.key.clone());
            let mut http = Http::new(url, bearer, ctl.timeout);
            http.open(ctl)?;
            op(&mut http, ctl)
        }
        Transport::Stdio { command, args } => {
            let local = local(target, command, args)?;
            let mut guard = local
                .lock()
                .map_err(|_| Failure::other("the server's session was left broken"))?;
            guard.open(ctl)?;
            op(&mut *guard, ctl)
        }
    }
}

fn message_of(v: &Value) -> Result<Value, Failure> {
    match v.get("error") {
        Some(e) => Err(Failure::other(
            e["message"]
                .as_str()
                .unwrap_or("the server reported an error"),
        )),
        None => Ok(v["result"].clone()),
    }
}

fn initialize_params() -> Value {
    json!({
        "protocolVersion": PROTOCOL,
        "capabilities": {},
        "clientInfo": {"name": "oido-desktop", "version": env!("CARGO_PKG_VERSION")},
    })
}

// ---- remote -------------------------------------------------------------------------------

struct Http {
    agent: ureq::Agent,
    url: String,
    bearer: Option<String>,
    session: Option<String>,
    next: u64,
}

impl Http {
    fn new(url: &str, bearer: Option<String>, timeout: Duration) -> Http {
        Http {
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(timeout))
                .http_status_as_error(false)
                .build()
                .into(),
            url: url.to_owned(),
            bearer,
            session: None,
            next: 1,
        }
    }

    /// Sends one message. A request (`wait` is its id) gets its reply back; a notification gets
    /// `None`.
    fn post(&mut self, body: &Value, wait: Option<u64>) -> Result<Option<Value>, Failure> {
        let mut call = self
            .agent
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        if let Some(b) = &self.bearer {
            call = call.header("Authorization", &format!("Bearer {b}"));
        }
        if let Some(s) = &self.session {
            call = call
                .header("Mcp-Session-Id", s)
                .header("MCP-Protocol-Version", PROTOCOL);
        }
        let mut resp = call
            .send(body.to_string().as_str())
            .map_err(|e| Failure::other(format!("the server could not be reached: {e}")))?;
        if let Some(s) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            self.session = Some(s.to_owned());
        }
        let status = resp.status().as_u16();
        let streamed = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.contains("text/event-stream"));
        match status {
            401 | 403 => return Err(Failure::Unauthorized),
            200..=299 => {}
            other => {
                let said = resp.body_mut().read_to_string().unwrap_or_default();
                let said: String = said.chars().take(200).collect();
                return Err(Failure::other(format!(
                    "the server answered HTTP {other}{}",
                    if said.trim().is_empty() {
                        String::new()
                    } else {
                        format!(": {}", said.trim())
                    }
                )));
            }
        }
        let Some(id) = wait else {
            return Ok(None);
        };
        let reader = resp.body_mut().as_reader();
        let reply = if streamed {
            from_stream(reader, id)
        } else {
            let mut text = String::new();
            BufReader::new(reader)
                .read_to_string(&mut text)
                .map_err(|e| Failure::other(format!("the reply could not be read: {e}")))?;
            serde_json::from_str::<Value>(&text)
                .ok()
                .filter(|v| v["id"].as_u64() == Some(id))
        };
        reply
            .map(Some)
            .ok_or_else(|| Failure::other("the server ended without answering"))
    }
}

impl Rpc for Http {
    fn open(&mut self, ctl: &Ctl) -> Result<(), Failure> {
        self.request("initialize", initialize_params(), ctl)?;
        self.post(
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            None,
        )?;
        Ok(())
    }

    fn request(&mut self, method: &str, params: Value, ctl: &Ctl) -> Result<Value, Failure> {
        if ctl.cancel.load(Ordering::Relaxed) {
            return Err(Failure::other("stopped before the server answered"));
        }
        let id = self.next;
        self.next += 1;
        let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let reply = self.post(&body, Some(id))?.unwrap_or(Value::Null);
        message_of(&reply)
    }
}

/// Reads server-sent events until the one that answers request `id`.
fn from_stream(reader: impl Read, id: u64) -> Option<Value> {
    let mut data: Vec<String> = Vec::new();
    for line in BufReader::new(reader).lines() {
        let line = line.ok()?;
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest).to_owned());
        } else if line.is_empty() && !data.is_empty() {
            let event = std::mem::take(&mut data).join("\n");
            if let Ok(v) = serde_json::from_str::<Value>(&event)
                && v["id"].as_u64() == Some(id)
            {
                return Some(v);
            }
        }
    }
    // A stream that ends without a closing blank line still may have said its piece.
    serde_json::from_str::<Value>(&data.join("\n"))
        .ok()
        .filter(|v| v["id"].as_u64() == Some(id))
}

// ---- local --------------------------------------------------------------------------------

/// A running local server.
struct Local {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    stderr: Arc<Mutex<String>>,
    next: u64,
    ready: bool,
}

type Pool = Mutex<HashMap<String, Arc<Mutex<Local>>>>;

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(Default::default)
}

/// The running local server for `target`, started if there is none.
fn local(target: &Target, command: &str, args: &[String]) -> Result<Arc<Mutex<Local>>, Failure> {
    let mut pool = pool()
        .lock()
        .map_err(|_| Failure::other("the list of running servers was left broken"))?;
    if let Some(running) = pool.get(&target.id) {
        return Ok(Arc::clone(running));
    }
    let started = Arc::new(Mutex::new(Local::start(command, args, &target.secrets)?));
    pool.insert(target.id.clone(), Arc::clone(&started));
    Ok(started)
}

impl Local {
    fn start(command: &str, args: &[String], secrets: &Secrets) -> Result<Local, Failure> {
        let mut cmd = Command::new(command);
        cmd.args(args).envs(&secrets.env);
        // Somewhere fixed rather than wherever the app was started from, where a relative path the
        // server writes to would otherwise land.
        if let Some(home) = dirs::home_dir() {
            cmd.current_dir(home);
        }
        cmd.stdin(Pipe::piped())
            .stdout(Pipe::piped())
            .stderr(Pipe::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Its own group, so ending it ends what it started too.
            cmd.process_group(0);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| Failure::other(format!("`{command}` could not be started: {e}")))?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            end(&mut child);
            return Err(Failure::other("the server's pipes could not be opened"));
        };
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let tail = Arc::new(Mutex::new(String::new()));
        let keep = Arc::clone(&tail);
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Ok(mut t) = keep.lock() {
                    t.push_str(&line);
                    t.push('\n');
                    if t.len() > STDERR_KEEP {
                        let mut from = t.len() - STDERR_KEEP;
                        while !t.is_char_boundary(from) {
                            from += 1;
                        }
                        t.drain(..from);
                    }
                }
            }
        });
        Ok(Local {
            child,
            stdin,
            lines,
            stderr: tail,
            next: 1,
            ready: false,
        })
    }

    fn send(&mut self, message: &Value) -> Result<(), Failure> {
        writeln!(self.stdin, "{message}")
            .and_then(|()| self.stdin.flush())
            .map_err(|_| Failure::other(format!("the server is not running{}", self.why())))
    }

    /// What the server last said on its error output, for explaining why it ended.
    fn why(&self) -> String {
        match self.stderr.lock() {
            Ok(t) if !t.trim().is_empty() => format!(": {}", t.trim()),
            _ => String::new(),
        }
    }
}

impl Rpc for Local {
    fn open(&mut self, ctl: &Ctl) -> Result<(), Failure> {
        if self.ready {
            return Ok(());
        }
        self.request("initialize", initialize_params(), ctl)?;
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
        self.ready = true;
        Ok(())
    }

    fn request(&mut self, method: &str, params: Value, ctl: &Ctl) -> Result<Value, Failure> {
        let id = self.next;
        self.next += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let deadline = Instant::now() + ctl.timeout;
        loop {
            if ctl.cancel.load(Ordering::Relaxed) {
                return Err(Failure::other("stopped before the server answered"));
            }
            let left = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| Failure::other("the server took too long to answer"))?;
            match self
                .lines
                .recv_timeout(left.min(Duration::from_millis(100)))
            {
                Ok(line) => {
                    let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if msg.get("method").is_some() {
                        // The server asks something of us (a ping, its roots, to sample): this
                        // client offers none of that, and an unanswered request would stall it.
                        if let Some(their_id) = msg.get("id") {
                            self.send(&json!({
                                "jsonrpc": "2.0",
                                "id": their_id,
                                "error": {"code": -32601, "message": "not supported"},
                            }))?;
                        }
                    } else if msg["id"].as_u64() == Some(id) {
                        return message_of(&msg);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Failure::other(format!("the server ended{}", self.why())));
                }
            }
        }
    }
}

impl Drop for Local {
    fn drop(&mut self) {
        end(&mut self.child);
    }
}

/// Ends a process and anything it started.
fn end(child: &mut Child) {
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{}", child.id())])
            .stdout(Pipe::null())
            .stderr(Pipe::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn a_result_is_read_as_text_and_pictures_are_named_not_dropped() {
        let r = json!({"content": [
            {"type": "text", "text": "one"},
            {"type": "image", "mimeType": "image/png", "data": "AAAA"},
            {"type": "resource", "resource": {"uri": "file:///a", "text": "two"}},
        ]});
        assert_eq!(
            render(&r),
            "one\n[image (image/png) returned; it cannot be shown here]\ntwo"
        );
        assert_eq!(render(&json!({"content": []})), "(no output)");
        assert_eq!(
            render(&json!({"content": [{"type": "text", "text": "bad"}], "isError": true})),
            "Error: bad"
        );
        assert_eq!(
            render(&json!({"structuredContent": {"a": 1}})),
            r#"{"a":1}"#
        );
    }

    #[test]
    fn long_output_is_cut_at_a_character() {
        let text = "é".repeat(OUTPUT_LIMIT);
        let out = render(&json!({"content": [{"type": "text", "text": text}]}));
        assert!(out.ends_with("[output cut]"));
        assert!(out.len() < OUTPUT_LIMIT + 20);
    }

    #[test]
    fn an_event_stream_is_read_to_the_event_that_answers() {
        let stream = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n\
                      event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\r\ndata: \"result\":{\"ok\":true}}\n\n";
        // A data field may be split over lines; the lines are joined with a newline.
        let v = from_stream(Cursor::new(stream), 7).unwrap();
        assert_eq!(v["result"]["ok"], true);
        assert!(from_stream(Cursor::new(stream), 8).is_none());
    }

    #[test]
    fn a_json_rpc_error_becomes_its_message() {
        let e = message_of(&json!({"id": 1, "error": {"code": -1, "message": "no such tool"}}));
        assert!(matches!(e, Err(Failure::Other(m)) if m == "no such tool"));
    }

    /// A local server is a program that answers JSON lines: here a few lines of `sh`.
    pub(crate) fn fake_script() -> String {
        r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18"}}\n' "$id";;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"Echoes.","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}}]}}\n' "$id";;
    *'"tools/call"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"pong"}]}}\n' "$id";;
  esac
done"#
        .to_owned()
    }

    fn fake(id: &str) -> Target {
        Target {
            id: id.to_owned(),
            transport: Transport::Stdio {
                command: "sh".into(),
                args: vec!["-c".into(), fake_script()],
            },
            secrets: Secrets::default(),
        }
    }

    /// What a fake HTTP server was asked: the request line, headers and body, as text.
    pub(crate) type Handler =
        dyn Fn(&str) -> (u16, Vec<(&'static str, String)>, String) + Send + Sync;

    /// A tiny HTTP server on a free port that answers each request with what `handler` says: the
    /// status, extra headers (`Content-Type` among them) and the body. Gives its base address.
    pub(crate) fn fake_http(handler: Box<Handler>) -> String {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let wanted = loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).into_owned();
                    if let Some(at) = text.find("\r\n\r\n") {
                        let len = text[..at]
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if raw.len() >= at + 4 + len {
                            break text;
                        }
                    }
                    if n == 0 {
                        break text;
                    }
                };
                let (status, headers, body) = handler(&wanted);
                let mut out = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                for (k, v) in headers {
                    out.push_str(&format!("{k}: {v}\r\n"));
                }
                let _ = write!(stream, "{out}\r\n{body}");
            }
        });
        base
    }

    /// A fake MCP server over HTTP that wants `Authorization: Bearer <token>`.
    pub(crate) fn mcp_handler(token: &'static str) -> Box<Handler> {
        Box::new(move |req| {
            if !req.contains(&format!("Bearer {token}"))
                && !req.contains(&format!("bearer {token}"))
            {
                return (401, vec![], String::new());
            }
            let body = req.split("\r\n\r\n").nth(1).unwrap_or_default();
            let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
            let id = v["id"].clone();
            let json = |result: Value| {
                (
                    200,
                    vec![("Content-Type", "application/json".to_owned())],
                    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string(),
                )
            };
            match v["method"].as_str() {
                Some("initialize") => (
                    200,
                    vec![
                        ("Content-Type", "application/json".to_owned()),
                        ("Mcp-Session-Id", "s1".to_owned()),
                    ],
                    json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": PROTOCOL}})
                        .to_string(),
                ),
                Some("notifications/initialized") => (202, vec![], String::new()),
                // Listed as a server-sent event, after an unrelated one.
                Some("tools/list") => (
                    200,
                    vec![("Content-Type", "text/event-stream".to_owned())],
                    format!(
                        "event: message\ndata: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\"}}\n\nevent: message\ndata: {}\n\n",
                        json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [{"name": "echo", "description": "Echoes.", "inputSchema": {"type": "object"}}]}})
                    ),
                ),
                Some("tools/call") => json(json!({"content": [{"type": "text", "text": "pong"}]})),
                _ => (404, vec![], String::new()),
            }
        })
    }

    #[test]
    fn a_remote_server_is_listed_and_called_with_its_key_and_refused_without_one() {
        let base = fake_http(mcp_handler("k1"));
        let never = AtomicBool::new(false);
        let target = |key: Option<&str>| Target {
            id: "t-http".into(),
            transport: Transport::Http {
                url: format!("{base}/mcp"),
            },
            secrets: Secrets {
                key: key.map(str::to_owned),
                ..Secrets::default()
            },
        };
        let tools = list_tools(target(Some("k1")), &never).result.unwrap();
        assert_eq!(tools.len(), 1);
        let told = call_tool(target(Some("k1")), "echo", json!({"text": "hi"}), &never)
            .result
            .unwrap();
        assert_eq!(told, "pong");
        let why = list_tools(target(None), &never).result.unwrap_err();
        assert!(why.contains("did not accept"), "{why}");
        let why = list_tools(target(Some("wrong")), &never)
            .result
            .unwrap_err();
        assert!(why.contains("did not accept"), "{why}");
    }

    #[test]
    fn a_local_server_is_started_once_listed_called_and_ended() {
        let never = AtomicBool::new(false);
        let tools = list_tools(fake("t-local"), &never).result.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        let told = call_tool(fake("t-local"), "echo", json!({}), &never)
            .result
            .unwrap();
        assert_eq!(told, "pong");
        assert!(pool().lock().unwrap().contains_key("t-local"));
        stop("t-local");
        assert!(!pool().lock().unwrap().contains_key("t-local"));
    }

    #[test]
    fn a_local_server_that_cannot_start_says_so_and_a_stopped_call_gives_up() {
        let never = AtomicBool::new(false);
        let bad = Target {
            id: "t-missing".into(),
            transport: Transport::Stdio {
                command: "/nonexistent/oido-mcp".into(),
                args: vec![],
            },
            secrets: Secrets::default(),
        };
        let why = list_tools(bad, &never).result.unwrap_err();
        assert!(why.contains("could not be started"), "{why}");
        let stopped = AtomicBool::new(true);
        let why = list_tools(fake("t-stopped"), &stopped).result.unwrap_err();
        assert!(why.contains("stopped"), "{why}");
        stop("t-stopped");
    }
}

//! Helpers shared by tests: a fake provider server and a throwaway database file.
#![allow(clippy::expect_used)] // a test helper that cannot get a port or a file should stop the test

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc;

use crate::app::App;
use crate::store::Store;
use crate::vault::Vault;

/// A ChatGPT access token in the shape the backend issues (header, claims, signature), naming
/// `account` if there is one.
pub fn access_token(account: Option<&str>) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let claims = match account {
        Some(id) => serde_json::json!({"https://api.openai.com/auth": {"chatgpt_account_id": id}}),
        None => serde_json::json!({"sub": "someone"}),
    };
    let part = |v: &serde_json::Value| URL_SAFE_NO_PAD.encode(v.to_string());
    format!(
        "{}.{}.sig",
        part(&serde_json::json!({"alg": "RS256"})),
        part(&claims)
    )
}

/// Adds a reply to the open chat in which the agent asked for tool `name` as call `id`, and gives
/// the chat's id. The call has no result yet.
pub fn ask_for(app: &mut App, name: &str, id: &str, args: &str) -> String {
    use crate::model::{Message, Outcome, ToolCall};
    let mut asked = Message::reply("", None, Outcome::Done);
    if let Message::Assistant { tools, .. } = &mut asked {
        tools.push(ToolCall {
            name: name.into(),
            id: id.into(),
            args: args.into(),
            ..Default::default()
        });
    }
    let chat = app
        .roster
        .current_mut()
        .expect("an agent is open")
        .chat_mut();
    chat.messages.push(asked);
    chat.id.clone()
}

/// What the open chat's last reply says call `id` gave back, once it has.
pub fn result_of(app: &App, id: &str) -> Option<String> {
    use crate::model::Message;
    match app.current()?.chat().messages.last()? {
        Message::Assistant { tools, .. } => tools.iter().find(|t| t.id == id)?.output.clone(),
        _ => None,
    }
}

/// Serves one canned JSON reply per connection, in order, and reports each request it saw
/// (lowercased).
pub fn serve(replies: Vec<(u16, &'static str)>) -> (String, mpsc::Receiver<String>) {
    serve_as(
        replies
            .into_iter()
            .map(|(status, body)| (status, "application/json", body))
            .collect(),
    )
}

/// Like [`serve`], with each reply's content type given (`text/event-stream` for a streamed
/// answer). The whole request is read, body included, before it answers.
pub fn serve_as(
    replies: Vec<(u16, &'static str, &'static str)>,
) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let base = format!("http://{}", listener.local_addr().expect("an address"));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for (status, kind, body) in replies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let (mut buf, mut req) = ([0u8; 8192], Vec::new());
            let mut wanted = None;
            loop {
                let n = stream.read(&mut buf).unwrap_or(0);
                req.extend_from_slice(&buf[..n]);
                if n == 0 {
                    break;
                }
                if wanted.is_none()
                    && let Some(end) = req.windows(4).position(|w| w == b"\r\n\r\n")
                {
                    let head = String::from_utf8_lossy(&req[..end]).to_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    wanted = Some(end + 4 + len);
                }
                if wanted.is_some_and(|w| req.len() >= w) {
                    break;
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&req).to_lowercase());
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    (base, rx)
}

/// A throwaway database file, removed when dropped.
pub struct TempDb(pub PathBuf);

impl TempDb {
    pub fn new() -> TempDb {
        TempDb(std::env::temp_dir().join(format!("oido-app-test-{}.db", uuid::Uuid::new_v4())))
    }

    /// The app on this file, with a fixed vault key so no real keyring is touched.
    pub fn open(&self) -> App {
        self.open_with(Vault::fixed([9; 32]))
    }

    /// The app on this file with the sample agents in it, saved, so a later `open` finds them.
    pub fn open_with_samples(&self) -> App {
        let mut app = self.open();
        crate::demo::load_samples(&mut app);
        if let Some(db) = &app.store {
            db.save_roster(&app.roster.list).expect("the samples save");
        }
        app
    }

    pub fn open_with(&self, vault: Vault) -> App {
        App::new(Some(Store::open(&self.0, vault).expect("a store")))
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

/// Connects Anthropic with one model and makes it the default, so agents can answer.
pub fn ready(app: &mut App) {
    use crate::models::{ModelInfo, ModelRef};
    app.providers.connect("anthropic");
    app.providers.set_models(
        "anthropic",
        vec![ModelInfo {
            id: "claude-test".into(),
            name: "Claude Test".into(),
            caps: Default::default(),
            context: None,
        }],
        1,
    );
    app.providers.default_model = Some(ModelRef::new("anthropic", "claude-test"));
}

/// The app (no database) with the sample agents, Atlas first and waiting on an approval.
pub fn samples() -> App {
    let mut app = App::new(None);
    crate::demo::load_samples(&mut app);
    app
}

/// What [`app_with_server`]'s server says it runs with at `/props`.
pub const SERVER_CONTEXT: u32 = 32_768;

/// The sample agents on a database file, with a server connected that answers `replies` after
/// listing its models and its context, and the open agent using that server's model.
pub fn app_with_server(
    db: &TempDb,
    replies: Vec<(u16, &'static str, &'static str)>,
) -> (App, mpsc::Receiver<String>) {
    use crate::models::ModelRef;
    use crate::providers::NEW_SERVER;
    let mut app = db.open_with_samples();
    let mut all = vec![(
        200,
        "application/json",
        r#"{"data":[{"id":"llama-3.3-70b"}]}"#,
    )];
    all.push((
        200,
        "application/json",
        r#"{"default_generation_settings":{"n_ctx":32768}}"#,
    ));
    all.extend(replies);
    let (base, seen) = serve_as(all);
    app.providers.toggle(NEW_SERVER);
    app.providers.name = rune::TextEdit::new("Box");
    app.providers.url = rune::TextEdit::new(format!("{base}/v1"));
    app.providers.key = rune::TextEdit::new("sk-box");
    app.save_server(NEW_SERVER);
    let _ = seen.recv().expect("the server saw the model list request");
    let _ = seen.recv().expect("the server saw the context request");
    app.set_model(Some(ModelRef::new("llama-server:box", "llama-3.3-70b")));
    (app, seen)
}

/// The content type of a streamed answer, for [`serve_as`].
pub const EVENTS: &str = "text/event-stream";

/// A streamed reply that is just `text`.
pub fn text_stream(text: &str) -> &'static str {
    let chunk =
        serde_json::json!({"choices": [{"delta": {"content": text}, "finish_reason": "stop"}]});
    Box::leak(format!("data: {chunk}\n\ndata: [DONE]\n\n").into_boxed_str())
}

/// A streamed reply in which the model calls the tool `name` (call id `c1`).
pub fn call_stream(name: &str, args: serde_json::Value) -> &'static str {
    let chunk = serde_json::json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "c1", "function": {"name": name, "arguments": args.to_string()}}]}, "finish_reason": "tool_calls"}]});
    Box::leak(format!("data: {chunk}\n\ndata: [DONE]\n\n").into_boxed_str())
}

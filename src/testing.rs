//! Helpers shared by tests: a fake provider server and a throwaway database file.
#![allow(clippy::expect_used)] // a test helper that cannot get a port or a file should stop the test

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc;

use crate::app::App;
use crate::store::Store;
use crate::vault::Vault;

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

/// The sample agents on a database file, with a server connected that answers `replies` after
/// listing its models, and the open agent using that server's model.
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
    all.extend(replies);
    let (base, seen) = serve_as(all);
    app.providers.toggle(NEW_SERVER);
    app.providers.name = rune::TextEdit::new("Box");
    app.providers.url = rune::TextEdit::new(format!("{base}/v1"));
    app.providers.key = rune::TextEdit::new("sk-box");
    app.save_server(NEW_SERVER);
    let _ = seen.recv().expect("the server saw the model list request");
    app.set_model(Some(ModelRef::new("llama-server:box", "llama-3.3-70b")));
    (app, seen)
}

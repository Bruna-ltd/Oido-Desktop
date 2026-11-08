//! Helpers shared by tests: a fake provider server and a throwaway database file.
#![allow(clippy::expect_used)] // a test helper that cannot get a port or a file should stop the test

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc;

use crate::app::App;
use crate::store::Store;
use crate::vault::Vault;

/// Serves one canned reply per connection, in order, and reports each request it saw (lowercased).
pub fn serve(replies: Vec<(u16, &'static str)>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let base = format!("http://{}", listener.local_addr().expect("an address"));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for (status, body) in replies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let (mut buf, mut req) = ([0u8; 8192], Vec::new());
            loop {
                let n = stream.read(&mut buf).unwrap_or(0);
                req.extend_from_slice(&buf[..n]);
                if n == 0 || req.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&req).to_lowercase());
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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

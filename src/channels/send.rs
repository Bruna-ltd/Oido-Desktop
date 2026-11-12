//! The `send_file` tool: an agent sends a file from its folder to the person it talks to on a
//! channel, privately: a picture as a photo, anything else as a file. Offered only in chats
//! carried on from a channel. It runs as a job, like a command: the chat is busy until the upload
//! ends.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use super::{Creds, Kind, Place, slack, telegram};
use crate::app::App;
use crate::command::Finish;
use crate::completion::ToolSpec;

pub const NAME: &str = "send_file";

/// The tool as offered in a chat carried on from `kind`.
pub fn spec(kind: Kind) -> ToolSpec {
    ToolSpec {
        name: NAME.into(),
        description: format!(
            "Send a file from your working folder to the person you are talking to on {}: a picture (PNG, JPEG, WebP) shows as a photo, anything else comes as a file to download. Up to {} MB. Use it when they ask for a file, or when a file you made answers them better than text.",
            kind.title(),
            max_file(kind) / (1024 * 1024)
        ),
        schema: json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "The file, relative to the working folder."},
                "caption": {"type": "string", "description": "A line to send with it."}
            },
            "required": ["path"]
        }),
    }
}

impl App {
    /// Starts sending the file a `send_file` call asks for. One that cannot be sent is answered
    /// at once with why, and the next job starts.
    pub(crate) fn start_send_file(&mut self, sid: &str, call: &str, args: &str) {
        match self.file_to_send(sid, args) {
            Err(why) => {
                self.resolve(sid, call, format!("Error: {why}"), true);
                self.start_next_command(sid);
            }
            Ok(job) => {
                let (sid, call) = (sid.to_owned(), call.to_owned());
                let cancel = Arc::new(AtomicBool::new(false));
                self.commands.insert(sid.clone(), Arc::clone(&cancel));
                self.spawn(
                    move || (job.send(), cancel.load(Ordering::Relaxed)),
                    move |app, (said, stopped)| {
                        let finish = if stopped {
                            Finish::Stopped("Stopped; it may have been sent already.".into())
                        } else {
                            Finish::Done(said)
                        };
                        app.command_done(&sid, &call, finish);
                    },
                );
            }
        }
    }

    fn file_to_send(&self, sid: &str, args: &str) -> Result<Upload, String> {
        let kind = self.channels.kind_of(sid).ok_or(
            "this chat is not carried on from a channel, so there is no one to send it to.",
        )?;
        if self.channels.files_go(sid).is_none() {
            return Err(format!(
                "files go to the person privately, and this chat is not carried on privately on {}, or {} takes no files from agents.",
                kind.title(),
                kind.title()
            ));
        }
        let c = self.channels.get(kind);
        let user = c.user_of(sid).ok_or("no one carries this chat on now.")?;
        let creds = c.creds.clone().ok_or_else(|| {
            format!(
                "the {} bot is not listening now, so nothing can be sent.",
                kind.title()
            )
        })?;
        let v: Value = serde_json::from_str(args).map_err(|_| "the arguments were not JSON.")?;
        let given = v.get("path").and_then(Value::as_str).unwrap_or_default();
        let path = crate::files::resolve(&self.workdir_of(sid), given)?;
        let meta = std::fs::metadata(&path).map_err(|_| format!("{given} does not exist."))?;
        if !meta.is_file() {
            return Err(format!("{given} is not a file."));
        }
        let most = max_file(kind);
        if meta.len() > most {
            return Err(format!(
                "{given} is {} MB; {} takes files up to {} MB.",
                meta.len() / (1024 * 1024),
                kind.title(),
                most / (1024 * 1024)
            ));
        }
        let name = path
            .file_name()
            .map_or_else(|| "file".to_owned(), |n| n.to_string_lossy().into_owned());
        Ok(Upload {
            kind,
            creds,
            api: c.api.clone(),
            // Files go to the person privately, never into a group's thread.
            to: Place::private(&user),
            path,
            name,
            caption: v.get("caption").and_then(Value::as_str).map(str::to_owned),
        })
    }
}

/// The biggest file a bot can send on `kind`, in bytes.
fn max_file(kind: Kind) -> u64 {
    match kind {
        Kind::Telegram => telegram::MAX_FILE,
        Kind::Slack => slack::MAX_FILE,
        // Not yet: a file would go through the running client, not a stateless call.
        Kind::WhatsApp => 0,
    }
}

/// Whether agents can send files on `kind`.
pub(super) fn takes_files(kind: Kind) -> bool {
    max_file(kind) > 0
}

/// A file checked and ready to go.
struct Upload {
    kind: Kind,
    creds: Creds,
    /// Another API address than the channel's own, for tests.
    api: Option<String>,
    to: Place,
    path: PathBuf,
    name: String,
    caption: Option<String>,
}

impl Upload {
    /// Sends it; blocks. Says how it went, for the model.
    fn send(&self) -> String {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) => return format!("Error: {} could not be read: {e}.", self.name),
        };
        let sent = match &self.creds {
            Creds::Telegram(token) => telegram::upload(
                self.api.as_deref().unwrap_or(telegram::API),
                token,
                &self.to,
                &self.name,
                &bytes,
                self.caption.as_deref(),
            )
            .map_err(|e| e.to_string()),
            Creds::Slack(tokens) => slack::upload(
                self.api.as_deref().unwrap_or(slack::API),
                &tokens.bot,
                &self.to,
                &self.name,
                &bytes,
                self.caption.as_deref(),
            )
            .map_err(|e| e.to_string()),
            Creds::WhatsApp(_) => Err("WhatsApp takes no files from agents yet".to_owned()),
        };
        match sent {
            Ok(()) => format!("Sent {} to them on {}.", self.name, self.kind.title()),
            Err(e) => format!("Error: {} was not sent: {e}.", self.name),
        }
    }
}

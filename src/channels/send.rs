//! What the channel tools send (see [`super::tools`]): a message or a file, to someone on a
//! channel. Each runs as a job, like a command: the chat is busy until it has gone.

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use super::{Creds, Kind, Out, Place, Reply, slack, telegram, whatsapp};

/// The biggest file a bot can send on `kind`, in bytes.
pub(super) fn max_file(kind: Kind) -> u64 {
    match kind {
        Kind::Telegram => telegram::MAX_FILE,
        Kind::Slack => slack::MAX_FILE,
        Kind::WhatsApp => whatsapp::MAX_FILE,
    }
}

/// Whether agents can send files on `kind`.
pub(super) fn takes_files(kind: Kind) -> bool {
    max_file(kind) > 0
}

/// A file checked and ready to go.
pub(super) struct Upload {
    pub(super) kind: Kind,
    pub(super) creds: Creds,
    /// Another API address than the channel's own, for tests.
    pub(super) api: Option<String>,
    pub(super) to: Place,
    /// Who it goes to, in words.
    pub(super) label: String,
    pub(super) path: PathBuf,
    pub(super) name: String,
    pub(super) caption: Option<String>,
    /// The running client's outbox, where files go through it (WhatsApp).
    pub(super) outbox: Option<Sender<Out>>,
}

/// How long a file sent through the running client may take to go.
const CLIENT_SEND_WAIT: Duration = Duration::from_secs(300);
/// How long a message may take to go: the channel's thread sends in order, so it can wait behind
/// a reply that is going out.
const NOTE_WAIT: Duration = Duration::from_secs(60);

/// A message checked and ready to go. It goes through the channel's own thread, in order with
/// everything else the channel sends.
pub(super) struct Note {
    pub(super) kind: Kind,
    pub(super) to: Place,
    /// Who it goes to, in words.
    pub(super) label: String,
    pub(super) text: String,
    pub(super) outbox: Sender<Out>,
}

impl Note {
    /// Sends it; blocks. Says how it went, for the model.
    pub(super) fn send(&self) -> String {
        let (tx, rx) = mpsc::channel();
        let handed = self.outbox.send(Out::Tell {
            to: self.to.clone(),
            text: self.text.clone(),
            reply: Reply::new(tx),
        });
        if handed.is_err() {
            return format!(
                "Error: it was not sent: the {} bot stopped.",
                self.kind.title()
            );
        }
        match rx.recv_timeout(NOTE_WAIT) {
            Ok(Ok(())) => format!("Sent to {} on {}.", self.label, self.kind.title()),
            Ok(Err(e)) => format!("Error: it was not sent: {e}."),
            Err(_) => format!(
                "Error: {} did not confirm it in time; it may still arrive.",
                self.kind.title()
            ),
        }
    }
}

impl Upload {
    /// Sends it; blocks. Says how it went, for the model.
    pub(super) fn send(&self) -> String {
        let sent = match &self.creds {
            // The client reads the file itself.
            Creds::WhatsApp(_) => self.through_client(),
            Creds::Telegram(_) | Creds::Slack(_) => match std::fs::read(&self.path) {
                Ok(bytes) => self.upload(&bytes),
                Err(e) => return format!("Error: {} could not be read: {e}.", self.name),
            },
        };
        self.said(sent)
    }

    /// Sends it in one call, where the channel has one.
    fn upload(&self, bytes: &[u8]) -> Result<(), String> {
        match &self.creds {
            Creds::Telegram(token) => telegram::upload(
                self.api.as_deref().unwrap_or(telegram::API),
                token,
                &self.to,
                &self.name,
                bytes,
                self.caption.as_deref(),
            )
            .map_err(|e| e.to_string()),
            Creds::Slack(tokens) => slack::upload(
                self.api.as_deref().unwrap_or(slack::API),
                &tokens.bot,
                &self.to,
                &self.name,
                bytes,
                self.caption.as_deref(),
            )
            .map_err(|e| e.to_string()),
            Creds::WhatsApp(_) => self.through_client(),
        }
    }

    /// Hands the file to the running client and waits for it to go.
    fn through_client(&self) -> Result<(), String> {
        let outbox = self.outbox.as_ref().ok_or("the link is not running now")?;
        let (tx, rx) = mpsc::channel();
        outbox
            .send(Out::File {
                to: self.to.clone(),
                path: self.path.clone(),
                name: self.name.clone(),
                caption: self.caption.clone(),
                reply: Reply::new(tx),
            })
            .map_err(|_| "the link is not running now".to_owned())?;
        rx.recv_timeout(CLIENT_SEND_WAIT)
            .map_err(|_| "it did not go in time, or the link stopped".to_owned())?
    }

    fn said(&self, sent: Result<(), String>) -> String {
        match sent {
            Ok(()) => format!(
                "Sent {} to {} on {}.",
                self.name,
                self.label,
                self.kind.title()
            ),
            Err(e) => format!("Error: {} was not sent: {e}.", self.name),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn on_whatsapp(outbox: Option<Sender<Out>>) -> Upload {
        Upload {
            kind: Kind::WhatsApp,
            creds: Creds::WhatsApp(whatsapp::Key::parse(&"a".repeat(64)).unwrap()),
            api: None,
            to: Place::private("1@lid"),
            label: "Ada".into(),
            path: PathBuf::from("launch.mp4"),
            name: "launch.mp4".into(),
            caption: Some("Here".into()),
            outbox,
        }
    }

    #[test]
    fn on_whatsapp_a_file_goes_through_the_running_client_and_it_says_how_it_went() {
        let (tx, rx) = mpsc::channel();
        let client = std::thread::spawn(move || match rx.recv().unwrap() {
            Out::File {
                to,
                name,
                caption,
                reply,
                ..
            } => {
                assert_eq!((to.chat.as_str(), name.as_str()), ("1@lid", "launch.mp4"));
                assert_eq!(caption.as_deref(), Some("Here"));
                reply.answer(Ok(()));
            }
            other => panic!("{other:?}"),
        });
        assert_eq!(
            on_whatsapp(Some(tx)).send(),
            "Sent launch.mp4 to Ada on WhatsApp."
        );
        client.join().unwrap();

        let said = on_whatsapp(None).send();
        assert!(
            said.starts_with("Error:") && said.contains("not running"),
            "{said}"
        );
    }

    #[test]
    fn a_message_goes_through_the_channels_thread_and_it_says_how_it_went() {
        let (tx, rx) = mpsc::channel();
        let note = Note {
            kind: Kind::Telegram,
            to: Place::private("42"),
            label: "Ada".into(),
            text: "Done.".into(),
            outbox: tx,
        };
        let thread = std::thread::spawn(move || {
            for answer in [Ok(()), Err("chat not found".to_owned())] {
                match rx.recv().unwrap() {
                    Out::Tell { to, text, reply } => {
                        assert_eq!((to.chat.as_str(), text.as_str()), ("42", "Done."));
                        reply.answer(answer);
                    }
                    other => panic!("{other:?}"),
                }
            }
        });
        assert_eq!(note.send(), "Sent to Ada on Telegram.");
        assert_eq!(note.send(), "Error: it was not sent: chat not found.");
        thread.join().unwrap();
        assert!(note.send().contains("stopped"), "the thread is gone");
    }
}

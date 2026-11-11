//! Desktop notifications, through the desktop's notification server (`org.freedesktop.Notifications`
//! on the session bus): a reply that finished while the window was out of sight, a schedule's
//! fire. Clicking one opens its chat and brings the window forward.
//!
//! Notifications are posted from a thread of their own, so a slow server never stalls the window;
//! another thread hears the clicks. A chat has at most one notification: a newer one replaces it.

use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::Value;
use zbus::{MatchRule, message};

use crate::app::App;
use crate::launcher::APP_ID;

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
/// The action a click on the notification itself invokes.
const OPEN: &str = "default";
/// The longest body shown; a reply is cut to it.
const MAX_BODY: usize = 180;

/// One notification.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub title: String,
    pub body: String,
    /// The chat a click opens.
    pub chat: Option<String>,
}

/// Each shown notification's chat, by the server's id for it.
type Shown = Arc<Mutex<HashMap<u32, String>>>;

/// Posts `note`. Does nothing without a running app (tests, headless runs) or a notification server.
pub fn post(note: Note) {
    let Some(app) = crate::app::handle() else {
        return;
    };
    static SENDER: OnceLock<Option<Sender<Note>>> = OnceLock::new();
    if let Some(tx) = SENDER.get_or_init(|| start(app.clone())) {
        let _ = tx.send(note);
    }
}

/// Connects to the session bus and starts the threads that post notifications and hear clicks.
fn start(app: rune::AppHandle<App>) -> Option<Sender<Note>> {
    let conn = match Connection::session() {
        Ok(conn) => conn,
        Err(e) => {
            eprintln!("oido: no notifications (cannot reach the session bus): {e}");
            return None;
        }
    };
    let shown = Shown::default();
    {
        let (conn, shown) = (conn.clone(), Arc::clone(&shown));
        thread::spawn(move || {
            if let Err(e) = listen(&conn, &shown, &app) {
                eprintln!("oido: clicks on notifications will not open chats: {e}");
            }
        });
    }
    let (tx, rx) = mpsc::channel::<Note>();
    thread::spawn(move || {
        let mut warned = false;
        for note in rx {
            let replaces = match (note.chat.as_deref(), shown.lock()) {
                (Some(chat), Ok(shown)) => {
                    shown.iter().find(|(_, c)| *c == chat).map(|(id, _)| *id)
                }
                _ => None,
            };
            match show(&conn, &note, replaces.unwrap_or(0)) {
                Ok(id) => {
                    // Some servers give a replacement a new id: the old one is gone either way.
                    if let Some(chat) = note.chat
                        && let Ok(mut shown) = shown.lock()
                    {
                        shown.retain(|_, c| *c != chat);
                        shown.insert(id, chat);
                    }
                }
                Err(e) if !warned => {
                    warned = true;
                    eprintln!("oido: cannot show notifications: {e}");
                }
                Err(_) => {}
            }
        }
    });
    Some(tx)
}

/// Shows `note` in place of notification `replaces` (0 for none), returning its id.
fn show(conn: &Connection, note: &Note, replaces: u32) -> zbus::Result<u32> {
    let actions: &[&str] = match note.chat {
        Some(_) => &[OPEN, "Open"],
        None => &[],
    };
    let hints = HashMap::from([("desktop-entry", Value::from(APP_ID))]);
    let reply = conn.call_method(
        Some(SERVICE),
        PATH,
        Some(SERVICE),
        "Notify",
        &(
            "Oido",
            replaces,
            APP_ID,
            note.title.as_str(),
            escape(&note.body),
            actions,
            hints,
            -1i32,
        ),
    )?;
    reply.body().deserialize::<u32>()
}

/// Hears the server's signals for as long as the app runs: a click opens the notification's
/// chat, with the activation token the server sent just before it, so the window may take focus.
fn listen(conn: &Connection, shown: &Shown, app: &rune::AppHandle<App>) -> zbus::Result<()> {
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface(SERVICE)?
        .path(PATH)?
        .build();
    let mut tokens: HashMap<u32, String> = HashMap::new();
    for msg in MessageIterator::for_match_rule(rule, conn, None)? {
        let Ok(msg) = msg else { continue };
        let header = msg.header();
        let body = msg.body();
        match header.member().map(|m| m.as_str()) {
            Some("ActivationToken") => {
                if let Ok((id, token)) = body.deserialize::<(u32, String)>() {
                    tokens.insert(id, token);
                }
            }
            Some("ActionInvoked") => {
                let Ok((id, action)) = body.deserialize::<(u32, String)>() else {
                    continue;
                };
                let chat = shown.lock().ok().and_then(|s| s.get(&id).cloned());
                if action == OPEN
                    && let Some(chat) = chat
                {
                    app.update(move |s: &mut App, _| s.open_chat(&chat));
                    app.activate(tokens.remove(&id).unwrap_or_default());
                }
            }
            Some("NotificationClosed") => {
                if let Ok((id, _reason)) = body.deserialize::<(u32, u32)>() {
                    if let Ok(mut shown) = shown.lock() {
                        shown.remove(&id);
                    }
                    tokens.remove(&id);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// A reply as a notification's body: its words on one line, cut to [`MAX_BODY`] characters.
pub fn excerpt(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(MAX_BODY) {
        Some((cut, _)) => format!("{}…", flat[..cut].trim_end()),
        None => flat,
    }
}

/// `text` as plain text in a body the server may read as markup.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_becomes_one_short_line() {
        assert_eq!(excerpt("  Hello,\n\nworld  "), "Hello, world");
        let long = "word ".repeat(100);
        let cut = excerpt(&long);
        assert!(cut.ends_with('…'));
        assert!(
            cut.chars().count() <= MAX_BODY + 1,
            "{}",
            cut.chars().count()
        );
        assert_eq!(excerpt(&"é".repeat(300)).chars().count(), MAX_BODY + 1);
    }

    #[test]
    fn markup_shows_as_written() {
        assert_eq!(
            escape("a < b && <b>c</b>"),
            "a &lt; b &amp;&amp; &lt;b&gt;c&lt;/b&gt;"
        );
    }
}

/// Shows a real notification: `cargo test notify -- --ignored`, on a desktop.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod desktop {
    use super::*;

    #[test]
    #[ignore = "shows a notification on this desktop"]
    fn the_desktop_shows_it() {
        let conn = Connection::session().unwrap();
        let note = Note {
            title: "Oido".into(),
            body: excerpt("A test from <cargo test>: the reply is done & ready."),
            chat: Some("test".into()),
        };
        let id = show(&conn, &note, 0).unwrap();
        assert!(id > 0);
        assert!(
            show(&conn, &note, id).unwrap() > 0,
            "a chat's newer note replaces it"
        );
    }
}

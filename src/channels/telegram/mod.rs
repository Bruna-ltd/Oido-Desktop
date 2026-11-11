//! Telegram as a channel: connecting a bot by its token, the threads that hear and answer it
//! while the app runs, and Telegram's updates turned into what the core hears.
//!
//! Updates are fetched by long polling, so no public address is needed. Asking for updates past
//! one confirms it to Telegram, which then never sends it again: an update handed to the app is
//! done with, whatever the app makes of it. Messages go out from one thread, in order. Only
//! private chats are heard: in a group, others would read the replies.

mod api;
pub mod format;

pub use api::{API, MAX_FILE, Token, upload};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use api::{Error, Update, User};

use super::{Allowlist, Creds, Health, Heard, Kind, Link, Out, Person, Picture, Place, Status};
use crate::app::App;

/// How often "typing…" is said again while an agent works: Telegram shows it for 5 seconds.
const TYPING_EVERY: Duration = Duration::from_secs(4);
/// The longest wait between polls after a failure.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

impl App {
    /// Checks the typed token with Telegram and, if it works, saves it sealed and starts listening.
    pub fn connect_telegram(&mut self) {
        let Some(token) = Token::parse(&self.channels.telegram_token.text) else {
            self.channels.telegram.status = Status::Failed(
                "That is not a bot token. BotFather gives one like 123456789:AAH…, a number, a colon, then letters.".into(),
            );
            return;
        };
        self.channels.telegram.status = Status::Connecting;
        let job_token = token.clone();
        self.spawn(
            move || {
                let me = api::me(API, &job_token);
                if me.is_ok() {
                    api::set_commands(API, &job_token);
                }
                me
            },
            move |app, me| app.telegram_checked(token, me),
        );
    }

    fn telegram_checked(&mut self, token: Token, me: Result<User, Error>) {
        let me = match me {
            Ok(me) if me.is_bot => me,
            Ok(_) => {
                self.channels.telegram.status = Status::Failed("That token is not a bot's.".into());
                return;
            }
            Err(e) => {
                self.channels.telegram.status = Status::Failed(format!("Could not connect: {e}."));
                return;
            }
        };
        let name = me.username.unwrap_or(me.first_name);
        let secret = token.as_str().to_owned();
        if self.channel_connected(Kind::Telegram, &secret, name, Creds::Telegram(token)) {
            self.channels.telegram_token = rune::TextEdit::new("");
        }
    }
}

/// Starts the threads that hear the bot, answer through it, and keep "typing…" up. What they
/// hear goes to [`App::channel_heard`], tagged with `generation`, so a link that has since been
/// replaced is ignored.
pub fn start(
    app: rune::AppHandle<App>,
    token: Token,
    allowlist: Allowlist,
    generation: u64,
) -> Link {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    {
        let (app, token, stop) = (app.clone(), token.clone(), Arc::clone(&stop));
        thread::spawn(move || poll(&app, &token, &allowlist, &stop, generation));
    }
    {
        let (app, token) = (app.clone(), token);
        thread::spawn(move || deliver(&app, &token, &rx, generation));
    }
    {
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                thread::sleep(TYPING_EVERY);
                app.update(move |s: &mut App, _| s.channel_typing(Kind::Telegram, generation));
            }
        });
    }
    Link::new(stop, tx)
}

/// Fetches the picture in each private message from someone paired, so the app has it when it
/// hears the message, in order with the rest.
fn fetch_pictures(base: &str, token: &Token, allowlist: &Allowlist, list: &mut [Update]) {
    for u in list {
        let Some(m) = u.message.as_mut() else {
            continue;
        };
        let known = m.chat.kind == "private"
            && m.from.as_ref().is_some_and(|f| {
                allowlist
                    .read()
                    .is_ok_and(|p| p.contains(&f.id.to_string()))
            });
        if !known {
            continue;
        }
        if let Some((id, size)) = m.picture_file() {
            m.picture = Some(if size.is_some_and(|n| n > api::MAX_PICTURE) {
                Err(api::too_big())
            } else {
                api::download(base, token, id)
            });
        }
    }
}

fn poll(
    app: &rune::AppHandle<App>,
    token: &Token,
    allowlist: &Allowlist,
    stop: &AtomicBool,
    generation: u64,
) {
    let agent = api::agent(Duration::from_secs(api::POLL_SECS + 15));
    let mut offset = None;
    let mut backoff = Duration::from_secs(2);
    let mut health = None;
    let mut report = |h: Health| {
        if health.as_ref() != Some(&h) {
            health = Some(h.clone());
            app.update(move |s: &mut App, _| s.channel_health(Kind::Telegram, generation, h));
        }
    };
    while !stop.load(Ordering::Relaxed) {
        match api::updates(&agent, API, token, offset) {
            Ok(mut list) => {
                backoff = Duration::from_secs(2);
                report(Health::Listening);
                if let Some(last) = list.last() {
                    offset = Some(last.update_id + 1);
                }
                fetch_pictures(API, token, allowlist, &mut list);
                let heard: Vec<Heard> = list.into_iter().filter_map(heard).collect();
                if !heard.is_empty() && !stop.load(Ordering::Relaxed) {
                    app.update(move |s: &mut App, _| {
                        s.channel_heard(Kind::Telegram, generation, heard);
                    });
                }
            }
            Err(Error::Unauthorized) => {
                report(Health::Stopped(Error::Unauthorized.to_string()));
                return;
            }
            Err(Error::RetryAfter(s)) => thread::sleep(Duration::from_secs(s.min(60))),
            Err(e) => {
                report(Health::Retrying(e.to_string()));
                thread::sleep(backoff);
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}

/// Sends what the app hands over, in order, until the link is dropped. A failure is reported and
/// the rest still goes.
fn deliver(app: &rune::AppHandle<App>, token: &Token, rx: &Receiver<Out>, generation: u64) {
    let agent = api::agent(Duration::from_secs(30));
    for out in rx {
        if let Err(e) = api::send(&agent, API, token, &out)
            && matches!(out, Out::Say { .. })
        {
            let why = e.to_string();
            app.update(move |s: &mut App, _| {
                s.channel_send_failed(Kind::Telegram, generation, why);
            });
        }
    }
}

fn person(u: &User) -> Person {
    Person {
        id: u.id.to_string(),
        name: u.name(),
        first_name: u.first_name.clone(),
        handle: u.username.clone(),
    }
}

/// What the core hears in an update: a press, a command, or a message from a private chat.
/// Anything else (a group, a bot, an edit) is nothing.
fn heard(u: Update) -> Option<Heard> {
    if let Some(cb) = u.callback_query {
        return Some(Heard::Press {
            from: person(&cb.from),
            id: cb.id,
            data: cb.data.unwrap_or_default(),
            message: cb.message.map(|m| {
                let place = Place {
                    chat: m.chat.id.to_string(),
                    thread: None,
                };
                (place, m.message_id.to_string())
            }),
        });
    }
    let m = u.message?;
    let from = m
        .from
        .as_ref()
        .filter(|f| !f.is_bot && m.chat.kind == "private")
        .map(person)?;
    let place = Place {
        chat: m.chat.id.to_string(),
        thread: None,
    };
    let has_picture = m.picture_file().is_some();
    let text = m.text.or(m.caption).unwrap_or_default();
    if !has_picture && let Some(command) = text.trim().strip_prefix('/') {
        let (name, arg) = command
            .split_once(char::is_whitespace)
            .unwrap_or((command, ""));
        // `/agent@oido_bot` is how a command picked from the menu can arrive.
        let name = name.split('@').next().unwrap_or(name);
        return Some(Heard::Command {
            from,
            place,
            id: Some(m.message_id.to_string()),
            at: m.date,
            name: name.to_owned(),
            arg: arg.trim().to_owned(),
        });
    }
    let picture = has_picture.then(|| Picture {
        name: format!("telegram-photo-{}", m.message_id),
        bytes: m
            .picture
            .unwrap_or_else(|| Err("Telegram did not hand it over".to_owned())),
    });
    Some(Heard::Message {
        from,
        place,
        id: Some(m.message_id.to_string()),
        at: m.date,
        text,
        picture,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn update(json: &str) -> Update {
        serde_json::from_str(json).unwrap()
    }

    fn picture_update(from: i64) -> Update {
        update(&format!(
            r#"{{"update_id":1,"message":{{"message_id":3,"date":1,"chat":{{"id":{from},"type":"private"}},
            "from":{{"id":{from},"first_name":"Ada"}},"caption":"look",
            "photo":[{{"file_id":"small","file_size":10}},{{"file_id":"big","file_size":20}}]}}}}"#
        ))
    }

    #[test]
    fn private_messages_commands_and_presses_are_heard_and_groups_are_not() {
        let msg = |chat_type: &str, text: &str| {
            update(&format!(
                r#"{{"update_id":1,"message":{{"message_id":9,"date":100,"chat":{{"id":42,"type":"{chat_type}"}},
                "from":{{"id":42,"first_name":"Ada","last_name":"L","username":"ada"}},"text":"{text}"}}}}"#
            ))
        };
        let Some(Heard::Message {
            from, place, text, ..
        }) = heard(msg("private", "hello"))
        else {
            panic!("a message");
        };
        assert_eq!(
            (from.id.as_str(), from.name.as_str(), place),
            ("42", "Ada L", Place::private("42"))
        );
        assert_eq!(text, "hello");
        assert_eq!(heard(msg("group", "hello")), None);
        let Some(Heard::Command { name, arg, .. }) =
            heard(msg("private", "/agent@oido_bot  Atlas "))
        else {
            panic!("a command");
        };
        assert_eq!((name.as_str(), arg.as_str()), ("agent", "Atlas"));
        let press = update(
            r#"{"update_id":2,"callback_query":{"id":"cb","from":{"id":42,"first_name":"Ada"},
            "message":{"message_id":7,"date":1,"chat":{"id":42,"type":"private"}},"data":"agent:x"}}"#,
        );
        assert_eq!(
            heard(press),
            Some(Heard::Press {
                from: Person {
                    id: "42".into(),
                    name: "Ada".into(),
                    first_name: "Ada".into(),
                    handle: None,
                },
                id: "cb".into(),
                data: "agent:x".into(),
                message: Some((Place::private("42"), "7".into())),
            })
        );
    }

    #[test]
    fn a_picture_comes_with_its_caption_and_its_bytes_or_why_not() {
        let mut u = picture_update(42);
        u.message.as_mut().unwrap().picture = Some(Ok(b"JPEG".to_vec()));
        let Some(Heard::Message { text, picture, .. }) = heard(u) else {
            panic!("a message");
        };
        assert_eq!(text, "look");
        assert_eq!(
            picture,
            Some(Picture {
                name: "telegram-photo-3".into(),
                bytes: Ok(b"JPEG".to_vec()),
            })
        );
        let Some(Heard::Message { picture, .. }) = heard(picture_update(42)) else {
            panic!("a message");
        };
        assert!(picture.unwrap().bytes.is_err(), "not fetched: said so");
    }

    #[test]
    fn pictures_are_fetched_only_from_people_paired_and_only_up_to_the_limit() {
        let token = Token::parse("123:AAHabcdefghijklmnopqrstuvw").unwrap();
        let (base, seen) = crate::testing::serve(vec![
            (
                200,
                r#"{"ok":true,"result":{"file_path":"photos/big.jpg","file_size":20}}"#,
            ),
            (200, "JPEGBYTES"),
            (
                200,
                r#"{"ok":true,"result":{"file_path":"photos/huge.jpg","file_size":99999999}}"#,
            ),
        ]);
        let allowlist: Allowlist = Arc::default();
        let mut list = vec![picture_update(42)];
        fetch_pictures(&base, &token, &allowlist, &mut list);
        assert!(
            list[0].message.as_ref().unwrap().picture.is_none(),
            "a stranger's is left"
        );

        allowlist.write().unwrap().insert("42".into());
        fetch_pictures(&base, &token, &allowlist, &mut list);
        let m = list[0].message.as_ref().unwrap();
        assert_eq!(m.picture, Some(Ok(b"JPEGBYTES".to_vec())));
        assert!(
            seen.recv().unwrap().contains("\"file_id\":\"big\""),
            "the biggest size"
        );
        assert!(seen.recv().unwrap().starts_with("get /file/bot123:"));
        assert!(
            api::download(&base, &token, "huge")
                .unwrap_err()
                .contains("bigger than 10 MB")
        );
    }
}

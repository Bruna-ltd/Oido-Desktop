//! Slack as a channel: an app you make from [`MANIFEST`], connected with its two tokens, heard
//! through Socket Mode (a WebSocket the app opens, so Slack needs no public address) while the app
//! runs, and Slack's events turned into what the core hears.
//!
//! The bot answers direct messages, and @mentions in channels it was invited to, in the mention's
//! thread. From a mention the agent gets only what the person wrote, never the rest of the
//! thread, so others in the channel cannot steer it. Commands come as `/oido agent|new|stop|help`,
//! or as those words alone. A message being worked on wears a 👀 reaction.

mod api;
pub mod format;

pub use api::{API, MAX_FILE, Tokens, upload};

use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message as Frame, WebSocket};

use super::{Allowlist, Creds, Health, Heard, Kind, Link, Out, Person, Picture, Place, Status};
use crate::app::App;

/// How long a read on the socket waits before the listener looks whether it should stop.
const READ_WAIT: Duration = Duration::from_secs(1);
/// The longest wait between connection attempts after a failure.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// The Slack app, as Slack's "Create an app from a manifest" takes it: a bot that hears direct
/// messages and mentions through Socket Mode, the `/oido` command, and buttons.
pub const MANIFEST: &str = r#"{
  "display_information": {
    "name": "Oido",
    "description": "Talk to your Oido agents."
  },
  "features": {
    "app_home": {
      "home_tab_enabled": false,
      "messages_tab_enabled": true,
      "messages_tab_read_only_enabled": false
    },
    "bot_user": {
      "display_name": "Oido",
      "always_online": true
    },
    "slash_commands": [
      {
        "command": "/oido",
        "description": "Choose your agent, start fresh, or stop it",
        "usage_hint": "agent [name] | new | stop | help",
        "should_escape": false
      }
    ]
  },
  "oauth_config": {
    "scopes": {
      "bot": [
        "app_mentions:read",
        "chat:write",
        "commands",
        "files:read",
        "files:write",
        "im:history",
        "im:read",
        "im:write",
        "reactions:write",
        "users:read"
      ]
    }
  },
  "settings": {
    "event_subscriptions": {
      "bot_events": ["app_mention", "message.im"]
    },
    "interactivity": {
      "is_enabled": true
    },
    "org_deploy_enabled": false,
    "socket_mode_enabled": true,
    "token_rotation_enabled": false
  }
}"#;

impl App {
    /// Checks the typed tokens with Slack and, if both work, saves them sealed and starts
    /// listening.
    pub fn connect_slack(&mut self) {
        let c = &self.channels;
        let Some(tokens) = Tokens::parse(&c.slack_app_token.text, &c.slack_bot_token.text) else {
            self.channels.slack.status = Status::Failed(
                "Those are not the app's tokens: the app-level one starts with xapp-, the bot one with xoxb-.".into(),
            );
            return;
        };
        self.channels.slack.status = Status::Connecting;
        let job = tokens.clone();
        self.spawn(
            move || {
                let me = api::me(API, &job.bot)?;
                api::open_socket(API, &job.app)?;
                Ok(me)
            },
            move |app, me| app.slack_checked(tokens, me),
        );
    }

    fn slack_checked(&mut self, tokens: Tokens, me: Result<api::Me, api::Error>) {
        let me = match me {
            Ok(me) => me,
            Err(e) => {
                self.channels.slack.status = Status::Failed(format!("Could not connect: {e}."));
                return;
            }
        };
        let name = if me.team.is_empty() {
            me.name
        } else {
            format!("{} in {}", me.name, me.team)
        };
        let secret = tokens.secret();
        if self.channel_connected(Kind::Slack, &secret, name, Creds::Slack(tokens)) {
            self.channels.slack_app_token = rune::TextEdit::new("");
            self.channels.slack_bot_token = rune::TextEdit::new("");
        }
    }
}

/// Starts the threads that hear the app and answer through it. What they hear goes to
/// [`App::channel_heard`], tagged with `generation`, so a link that has since been replaced is
/// ignored.
pub fn start(
    app: rune::AppHandle<App>,
    tokens: Tokens,
    allowlist: Allowlist,
    generation: u64,
) -> Link {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    {
        let (app, tokens, stop) = (app.clone(), tokens.clone(), Arc::clone(&stop));
        thread::spawn(move || listen(&app, &tokens, &allowlist, &stop, generation));
    }
    thread::spawn(move || deliver(&app, &tokens, &rx, generation));
    Link::new(stop, tx)
}

fn listen(
    app: &rune::AppHandle<App>,
    tokens: &Tokens,
    allowlist: &Allowlist,
    stop: &AtomicBool,
    generation: u64,
) {
    let agent = api::agent(Duration::from_secs(30));
    let mut health = None;
    let mut report = |h: Health| {
        if health.as_ref() != Some(&h) {
            health = Some(h.clone());
            app.update(move |s: &mut App, _| s.channel_health(Kind::Slack, generation, h));
        }
    };
    let mut backoff = Duration::from_secs(2);
    let mut me = None;
    let mut names: HashMap<String, Person> = HashMap::new();
    while !stop.load(Ordering::Relaxed) {
        let opened = match &me {
            Some(_) => Ok(()),
            None => api::me(API, &tokens.bot).map(|m| me = Some(m)),
        }
        .and_then(|()| api::open_socket(API, &tokens.app));
        let url = match opened {
            Ok(url) => url,
            Err(api::Error::Unauthorized) => {
                report(Health::Stopped(api::Error::Unauthorized.to_string()));
                return;
            }
            Err(e) => {
                report(Health::Retrying(e.to_string()));
                thread::sleep(backoff);
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        let mut socket = match tungstenite::connect(url.as_str()) {
            Ok((socket, _)) => socket,
            Err(e) => {
                report(Health::Retrying(format!("cannot open Slack's socket: {e}")));
                thread::sleep(backoff);
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        backoff = Duration::from_secs(2);
        set_read_wait(&mut socket);
        let bot = me.as_ref().map_or("", |m| m.user_id.as_str()).to_owned();
        let mut lookup = |id: &str| {
            names
                .entry(id.to_owned())
                .or_insert_with(|| api::person(&agent, API, &tokens.bot, id))
                .clone()
        };
        // Only the pictures of people paired are fetched.
        let mut fetch = |user: &str, url: &str| {
            if allowlist.read().is_ok_and(|p| p.contains(user)) {
                api::download(&agent, &tokens.bot, url)
            } else {
                Err("they are not paired".to_owned())
            }
        };
        loop {
            if stop.load(Ordering::Relaxed) {
                let _ = socket.close(None);
                return;
            }
            let frame = match socket.read() {
                Ok(frame) => frame,
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                // Closed, or broken: connect again.
                Err(_) => break,
            };
            let Frame::Text(text) = frame else {
                if matches!(frame, Frame::Close(_)) {
                    break;
                }
                continue;
            };
            let Ok(envelope) = serde_json::from_str::<Value>(text.as_str()) else {
                continue;
            };
            // Slack wants each envelope acknowledged within seconds, before any work on it.
            if let Some(id) = envelope.get("envelope_id").and_then(Value::as_str) {
                let ack = json!({ "envelope_id": id }).to_string();
                if socket.send(Frame::Text(ack.into())).is_err() {
                    break;
                }
            }
            match envelope.get("type").and_then(Value::as_str) {
                Some("hello") => report(Health::Listening),
                // Slack is moving the connection: open a new one.
                Some("disconnect") => break,
                Some(kind) => {
                    let payload = &envelope["payload"];
                    if kind == "interactive" {
                        unbutton(&agent, &tokens.bot, payload);
                    }
                    if let Some(heard) = heard(kind, payload, &bot, &mut lookup, &mut fetch) {
                        app.update(move |s: &mut App, _| {
                            s.channel_heard(Kind::Slack, generation, vec![heard]);
                        });
                    }
                }
                None => {}
            }
        }
    }
}

/// Makes a read on `socket` give up after [`READ_WAIT`], so the listener can see it should stop.
fn set_read_wait(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>) {
    let tcp = match socket.get_mut() {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::Rustls(s) => &mut s.sock,
        _ => return,
    };
    let _ = tcp.set_read_timeout(Some(READ_WAIT));
}

/// Takes the buttons off the message a press came from, so none is pressed twice.
fn unbutton(agent: &ureq::Agent, bot: &api::Secret, payload: &Value) {
    let s = |v: &Value| v.as_str().map(str::to_owned);
    let (Some(channel), Some(ts)) = (
        s(&payload["container"]["channel_id"]).or_else(|| s(&payload["channel"]["id"])),
        s(&payload["container"]["message_ts"]).or_else(|| s(&payload["message"]["ts"])),
    ) else {
        return;
    };
    let message = &payload["message"];
    let text = message["text"].as_str().unwrap_or_default();
    api::unbutton(agent, API, bot, &channel, &ts, text, &message["blocks"]);
}

/// Sends what the app hands over, in order, until the link is dropped. A failure is reported and
/// the rest still goes.
fn deliver(app: &rune::AppHandle<App>, tokens: &Tokens, rx: &Receiver<Out>, generation: u64) {
    let agent = api::agent(Duration::from_secs(30));
    for out in rx {
        if let Err(e) = api::send(&agent, API, &tokens.bot, &out)
            && matches!(out, Out::Say { .. })
        {
            let why = e.to_string();
            app.update(move |s: &mut App, _| s.channel_send_failed(Kind::Slack, generation, why));
        }
    }
}

/// When a Slack message was sent, from its `ts` ("1700000000.123456"), in seconds.
fn sent_at(ts: &str) -> i64 {
    ts.split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// A command written as words alone ("new", "agent Atlas"), or `None`.
fn bare_command(text: &str) -> Option<(String, String)> {
    let (first, rest) = text
        .trim()
        .split_once(char::is_whitespace)
        .unwrap_or((text.trim(), ""));
    let name = first.to_lowercase();
    let known = matches!(name.as_str(), "new" | "stop" | "help") && rest.trim().is_empty()
        || name == "agent";
    known.then(|| (name, rest.trim().to_owned()))
}

/// Fetches a shared picture: whose it is, and where it is.
type Fetch<'a> = dyn FnMut(&str, &str) -> Result<Vec<u8>, String> + 'a;

/// The first picture shared with a message, fetched by `fetch` (from whom, from where), or why
/// it could not be.
fn picture_of(event: &Value, user: &str, fetch: &mut Fetch<'_>) -> Option<Picture> {
    let file = event["files"].as_array()?.iter().find(|f| {
        f["mimetype"]
            .as_str()
            .is_some_and(|m| m.starts_with("image/"))
    })?;
    let name = file["name"]
        .as_str()
        .map_or_else(|| "slack-picture".to_owned(), str::to_owned);
    let bytes = if file["size"].as_u64().is_some_and(|n| n > api::MAX_PICTURE) {
        Err(format!(
            "it is bigger than {} MB",
            api::MAX_PICTURE / (1024 * 1024)
        ))
    } else {
        match file["url_private_download"]
            .as_str()
            .or_else(|| file["url_private"].as_str())
        {
            Some(url) => fetch(user, url),
            None => Err("Slack gave no address for it".to_owned()),
        }
    };
    Some(Picture { name, bytes })
}

/// What the core hears in an envelope of `kind` with `payload`: a direct message (with a picture,
/// if one was shared), a mention, the `/oido` command, or a press. `bot` is the bot's own user id;
/// `who` names a user by id; `fetch` fetches a shared picture.
fn heard(
    kind: &str,
    payload: &Value,
    bot: &str,
    who: &mut dyn FnMut(&str) -> Person,
    fetch: &mut Fetch<'_>,
) -> Option<Heard> {
    let s = |v: &Value| v.as_str().map(str::to_owned);
    match kind {
        "events_api" => {
            let e = &payload["event"];
            let user = s(&e["user"]).filter(|u| u != bot)?;
            // Edits, joins, the bot's own messages and other bots' are not someone writing; a
            // message with a file shared is.
            let shared = e["subtype"].as_str() == Some("file_share");
            if (e.get("subtype").is_some() && !shared) || e.get("bot_id").is_some() {
                return None;
            }
            let ts = s(&e["ts"])?;
            let channel = s(&e["channel"])?;
            let place = match s(&e["type"])?.as_str() {
                "message" if e["channel_type"].as_str() == Some("im") => Place {
                    chat: channel,
                    thread: None,
                },
                // Answered in the mention's thread, or a new one under it.
                "app_mention" => Place {
                    chat: channel,
                    thread: Some(s(&e["thread_ts"]).unwrap_or_else(|| ts.clone())),
                },
                _ => return None,
            };
            let mention = format!("<@{bot}>");
            let text = s(&e["text"]).unwrap_or_default().replace(&mention, "");
            let text = text.trim().to_owned();
            let from = who(&user);
            let at = sent_at(&ts);
            let picture = picture_of(e, &user, fetch);
            Some(match bare_command(&text).filter(|_| picture.is_none()) {
                Some((name, arg)) => Heard::Command {
                    from,
                    place,
                    id: Some(ts),
                    at,
                    name,
                    arg,
                },
                None => Heard::Message {
                    from,
                    place,
                    id: Some(ts),
                    at,
                    text,
                    picture,
                },
            })
        }
        "slash_commands" => {
            let user = s(&payload["user_id"])?;
            let text = s(&payload["text"]).unwrap_or_default();
            let (name, arg) = text
                .trim()
                .split_once(char::is_whitespace)
                .unwrap_or((text.trim(), ""));
            let name = if name.is_empty() { "help" } else { name };
            Some(Heard::Command {
                from: who(&user),
                // A command is answered privately, wherever it was typed.
                place: Place::private(&user),
                id: None,
                at: crate::app::now(),
                name: name.to_lowercase(),
                arg: arg.trim().to_owned(),
            })
        }
        "interactive" if payload["type"].as_str() == Some("block_actions") => {
            let user = s(&payload["user"]["id"])?;
            let data = s(&payload["actions"][0]["value"])?;
            let channel =
                s(&payload["container"]["channel_id"]).or_else(|| s(&payload["channel"]["id"]))?;
            let ts =
                s(&payload["container"]["message_ts"]).or_else(|| s(&payload["message"]["ts"]))?;
            Some(Heard::Press {
                from: who(&user),
                // Slack's press has no id of its own to answer; a note goes to who pressed.
                id: user,
                data,
                message: Some((
                    Place {
                        chat: channel,
                        thread: None,
                    },
                    ts,
                )),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ada(id: &str) -> Person {
        Person {
            id: id.into(),
            name: "Ada Lovelace".into(),
            first_name: "Ada".into(),
            handle: Some("ada".into()),
        }
    }

    fn hear(kind: &str, payload: &str) -> Option<Heard> {
        let v: Value = serde_json::from_str(payload).unwrap();
        heard(kind, &v, "UBOT", &mut |id| ada(id), &mut |_, _| {
            Err("no fetching in this test".into())
        })
    }

    #[test]
    fn a_direct_message_is_heard_from_its_channel_and_the_bots_own_are_not() {
        let Some(Heard::Message {
            from,
            place,
            id,
            at,
            text,
            ..
        }) = hear(
            "events_api",
            r#"{"event":{"type":"message","channel_type":"im","user":"U1","text":"hi there","ts":"1700000000.000100","channel":"D9"}}"#,
        )
        else {
            panic!("a message");
        };
        assert_eq!(from.id, "U1");
        assert_eq!(
            place,
            Place {
                chat: "D9".into(),
                thread: None
            }
        );
        assert_eq!(
            (id.as_deref(), at, text.as_str()),
            (Some("1700000000.000100"), 1_700_000_000, "hi there")
        );
        for skipped in [
            r#"{"event":{"type":"message","channel_type":"im","user":"UBOT","text":"x","ts":"1.0","channel":"D9"}}"#,
            r#"{"event":{"type":"message","channel_type":"im","user":"U1","bot_id":"B1","text":"x","ts":"1.0","channel":"D9"}}"#,
            r#"{"event":{"type":"message","subtype":"message_changed","channel_type":"im","user":"U1","ts":"1.0","channel":"D9"}}"#,
            r#"{"event":{"type":"message","channel_type":"channel","user":"U1","text":"x","ts":"1.0","channel":"C1"}}"#,
        ] {
            assert_eq!(hear("events_api", skipped), None, "{skipped}");
        }
    }

    #[test]
    fn a_mention_is_answered_in_its_thread_with_only_what_was_written() {
        let Some(Heard::Message { place, text, .. }) = hear(
            "events_api",
            r#"{"event":{"type":"app_mention","user":"U1","text":"<@UBOT> sum this up","ts":"5.0","channel":"C1"}}"#,
        ) else {
            panic!("a message");
        };
        assert_eq!(
            place,
            Place {
                chat: "C1".into(),
                thread: Some("5.0".into())
            }
        );
        assert_eq!(text, "sum this up");
        let Some(Heard::Message { place, .. }) = hear(
            "events_api",
            r#"{"event":{"type":"app_mention","user":"U1","text":"<@UBOT> more","ts":"6.0","thread_ts":"5.0","channel":"C1"}}"#,
        ) else {
            panic!("a message");
        };
        assert_eq!(
            place.thread.as_deref(),
            Some("5.0"),
            "in the thread it came from"
        );
    }

    #[test]
    fn commands_come_as_oido_or_as_words_alone() {
        let Some(Heard::Command {
            name, arg, place, ..
        }) = hear(
            "slash_commands",
            r#"{"command":"/oido","text":"agent  Atlas","user_id":"U1","channel_id":"C1"}"#,
        )
        else {
            panic!("a command");
        };
        assert_eq!((name.as_str(), arg.as_str()), ("agent", "Atlas"));
        assert_eq!(place, Place::private("U1"), "answered privately");
        let Some(Heard::Command { name, .. }) = hear(
            "slash_commands",
            r#"{"command":"/oido","text":"","user_id":"U1"}"#,
        ) else {
            panic!("a command");
        };
        assert_eq!(name, "help");
        assert!(matches!(
            hear("events_api", r#"{"event":{"type":"message","channel_type":"im","user":"U1","text":"New","ts":"1.0","channel":"D9"}}"#),
            Some(Heard::Command { name, .. }) if name == "new"
        ));
        assert!(
            matches!(
                hear(
                    "events_api",
                    r#"{"event":{"type":"message","channel_type":"im","user":"U1","text":"new ideas for the launch","ts":"1.0","channel":"D9"}}"#
                ),
                Some(Heard::Message { .. })
            ),
            "a sentence is a message"
        );
    }

    #[test]
    fn a_press_names_who_pressed_and_the_message_it_was_under() {
        let pressed = hear(
            "interactive",
            r#"{"type":"block_actions","user":{"id":"U1"},"actions":[{"action_id":"b0-1","value":"b:3:1"}],
            "container":{"channel_id":"D9","message_ts":"7.0"}}"#,
        );
        assert_eq!(
            pressed,
            Some(Heard::Press {
                from: ada("U1"),
                id: "U1".into(),
                data: "b:3:1".into(),
                message: Some((
                    Place {
                        chat: "D9".into(),
                        thread: None
                    },
                    "7.0".into()
                )),
            })
        );
    }

    #[test]
    fn the_manifest_is_json_slack_takes_with_socket_mode_and_the_events_heard() {
        let v: Value = serde_json::from_str(MANIFEST).unwrap();
        assert_eq!(v["settings"]["socket_mode_enabled"], json!(true));
        let events = v["settings"]["event_subscriptions"]["bot_events"].to_string();
        assert!(events.contains("app_mention") && events.contains("message.im"));
        assert_eq!(
            v["features"]["slash_commands"][0]["command"],
            json!("/oido")
        );
    }

    #[test]
    fn a_shared_picture_comes_with_its_message_and_only_its_first_picture_is_fetched() {
        let payload: Value = serde_json::from_str(
            r#"{"event":{"type":"message","subtype":"file_share","channel_type":"im","user":"U1","text":"what is this?",
            "ts":"2.0","channel":"D9","files":[
              {"name":"notes.txt","mimetype":"text/plain","url_private_download":"https://files.slack.com/n"},
              {"name":"chart.png","mimetype":"image/png","size":20,"url_private_download":"https://files.slack.com/c"}]}}"#,
        )
        .unwrap();
        let mut asked = Vec::new();
        let Some(Heard::Message { text, picture, .. }) = heard(
            "events_api",
            &payload,
            "UBOT",
            &mut |id| ada(id),
            &mut |user, url| {
                asked.push((user.to_owned(), url.to_owned()));
                Ok(b"PNG".to_vec())
            },
        ) else {
            panic!("a message");
        };
        assert_eq!(text, "what is this?");
        assert_eq!(
            picture,
            Some(Picture {
                name: "chart.png".into(),
                bytes: Ok(b"PNG".to_vec()),
            })
        );
        assert_eq!(
            asked,
            [("U1".to_owned(), "https://files.slack.com/c".to_owned())]
        );
    }

    #[test]
    fn a_picture_too_big_is_not_fetched() {
        let payload: Value = serde_json::from_str(
            r#"{"event":{"type":"message","subtype":"file_share","channel_type":"im","user":"U1","text":"",
            "ts":"2.0","channel":"D9","files":[{"name":"huge.png","mimetype":"image/png","size":99999999,
            "url_private_download":"https://files.slack.com/h"}]}}"#,
        )
        .unwrap();
        let Some(Heard::Message { picture, .. }) = heard(
            "events_api",
            &payload,
            "UBOT",
            &mut |id| ada(id),
            &mut |_, _| panic!("not fetched"),
        ) else {
            panic!("a message");
        };
        assert!(
            picture
                .unwrap()
                .bytes
                .unwrap_err()
                .contains("bigger than 10 MB")
        );
    }
}

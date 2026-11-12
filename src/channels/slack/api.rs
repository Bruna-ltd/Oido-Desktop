//! Slack's Web API (<https://api.slack.com/web>): the calls the app makes, form-encoded, with the
//! bot token (or, to open a Socket Mode connection, the app token). Tokens never appear in an
//! error: errors are worded here.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::format;
use crate::channels::{Button, Out, Person, Place};
use crate::update::VERSION;

pub const API: &str = "https://slack.com/api";
/// The most buttons one row of them holds.
const ROW: usize = 25;
/// The longest a button's label may be.
const LABEL: usize = 75;
/// The reaction that marks a message as being worked on.
const WORKING: &str = "eyes";
/// The biggest picture fetched from someone, in bytes.
pub const MAX_PICTURE: u64 = 10 * 1024 * 1024;
/// The biggest file an agent sends, in bytes.
pub const MAX_FILE: u64 = 50 * 1024 * 1024;
/// Where Slack keeps files people share. The bot token goes nowhere else with a download.
const FILES: &str = "https://files.slack.com/";

/// A Slack token, kept out of debug output and wiped when dropped.
#[derive(Clone)]
pub struct Secret(Arc<Zeroizing<String>>);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

impl Secret {
    /// A token that starts with `prefix` and is made of what Slack's tokens are made of.
    fn parse(raw: &str, prefix: &str) -> Option<Secret> {
        let raw = raw.trim();
        let ok = raw.len() > prefix.len() + 10
            && raw.starts_with(prefix)
            && raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        ok.then(|| Secret(Arc::new(Zeroizing::new(raw.to_owned()))))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a Slack app signs in with: its app-level token, which opens Socket Mode, and its bot
/// token, which makes every other call.
#[derive(Clone, Debug)]
pub struct Tokens {
    pub app: Secret,
    pub bot: Secret,
}

impl Tokens {
    pub fn parse(app: &str, bot: &str) -> Option<Tokens> {
        Some(Tokens {
            app: Secret::parse(app, "xapp-")?,
            bot: Secret::parse(bot, "xoxb-")?,
        })
    }

    /// Both, as one string to seal.
    pub fn secret(&self) -> Zeroizing<String> {
        Zeroizing::new(json!({"app": self.app.as_str(), "bot": self.bot.as_str()}).to_string())
    }

    /// Both back from [`Tokens::secret`].
    pub fn from_secret(secret: &str) -> Option<Tokens> {
        let v: Value = serde_json::from_str(secret).ok()?;
        Tokens::parse(v.get("app")?.as_str()?, v.get("bot")?.as_str()?)
    }
}

/// Why a call did not go through, in words safe to show.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// Slack does not accept the token (revoked, mistyped, or the app was uninstalled).
    Unauthorized,
    /// Too many calls: wait this many seconds.
    RetryAfter(u64),
    /// Slack refused the call, with its reason.
    Refused(String),
    /// Slack could not be reached, or its answer could not be read.
    Network(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized => f.write_str("Slack does not accept this token"),
            Self::RetryAfter(s) => write!(f, "Slack asks to wait {s} seconds"),
            Self::Refused(why) => write!(f, "Slack refused: {why}"),
            Self::Network(why) => write!(f, "cannot reach Slack: {why}"),
        }
    }
}

pub(super) fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(format!("oido-desktop/{VERSION}"))
        .http_status_as_error(false)
        .build()
        .into()
}

fn encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

/// Calls `method` with `params`, signed with `token`, and reads its answer.
pub(super) fn call(
    agent: &ureq::Agent,
    base: &str,
    token: &Secret,
    method: &str,
    params: &[(&str, String)],
) -> Result<Value, Error> {
    let body = params
        .iter()
        .map(|(k, v)| format!("{k}={}", encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    let scrub = |e: &dyn std::fmt::Display| e.to_string().replace(token.as_str(), "…");
    let mut resp = agent
        .post(&format!("{base}/{method}"))
        .header("Authorization", &format!("Bearer {}", token.as_str()))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(body.as_bytes())
        .map_err(|e| Error::Network(scrub(&e)))?;
    let status = resp.status().as_u16();
    if status == 429 {
        let wait = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(5);
        return Err(Error::RetryAfter(wait));
    }
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| Error::Network(scrub(&e)))?;
    let answer: Value = serde_json::from_str(&text)
        .map_err(|_| Error::Network(format!("Slack answered {status} with something else")))?;
    if answer.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(answer);
    }
    let why = answer
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    Err(match why.as_str() {
        "invalid_auth"
        | "not_authed"
        | "account_inactive"
        | "token_revoked"
        | "token_expired"
        | "not_allowed_token_type" => Error::Unauthorized,
        "missing_scope" => Error::Refused(
            "missing_scope: the app needs the newest manifest from the Channels screen (update it in Slack's app settings and reinstall the app)".into(),
        ),
        _ => Error::Refused(why),
    })
}

/// Who the bot is.
#[derive(Clone, Debug, PartialEq)]
pub struct Me {
    pub user_id: String,
    pub name: String,
    pub team: String,
}

/// The bot behind `bot`, which proves the token works.
pub fn me(base: &str, bot: &Secret) -> Result<Me, Error> {
    let v = call(&agent(Duration::from_secs(15)), base, bot, "auth.test", &[])?;
    let field = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Ok(Me {
        user_id: field("user_id"),
        name: field("user"),
        team: field("team"),
    })
}

/// The address of a new Socket Mode connection, which proves the app token works.
pub fn open_socket(base: &str, app: &Secret) -> Result<String, Error> {
    let v = call(
        &agent(Duration::from_secs(15)),
        base,
        app,
        "apps.connections.open",
        &[],
    )?;
    v.get("url")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error::Network("Slack gave no address to connect to".into()))
}

/// Someone in the workspace, by id; just the id if Slack will not say.
pub(super) fn person(agent: &ureq::Agent, base: &str, bot: &Secret, id: &str) -> Person {
    let user = call(agent, base, bot, "users.info", &[("user", id.to_owned())])
        .ok()
        .and_then(|v| v.get("user").cloned())
        .unwrap_or(Value::Null);
    let profile = &user["profile"];
    let pick = |vals: &[&Value]| {
        vals.iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .find(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let name = pick(&[
        &profile["real_name"],
        &user["real_name"],
        &profile["display_name"],
        &user["name"],
    ])
    .unwrap_or_else(|| id.to_owned());
    Person {
        id: id.to_owned(),
        first_name: pick(&[&profile["first_name"]])
            .unwrap_or_else(|| name.split_whitespace().next().unwrap_or(&name).to_owned()),
        handle: pick(&[&user["name"]]),
        name,
    }
}

/// Fetches a file someone shared, from Slack's file host only, refusing one over
/// [`MAX_PICTURE`] or one that is not a picture (what Slack hands back when the app cannot read
/// files is a web page).
pub(super) fn download(agent: &ureq::Agent, bot: &Secret, url: &str) -> Result<Vec<u8>, String> {
    if !url.starts_with(FILES) {
        return Err("it is not on Slack's file host".into());
    }
    let scrub = |e: &dyn std::fmt::Display| e.to_string().replace(bot.as_str(), "…");
    let mut resp = agent
        .get(url)
        .header("Authorization", &format!("Bearer {}", bot.as_str()))
        .call()
        .map_err(|e| scrub(&e))?;
    let status = resp.status().as_u16();
    let kind = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if status != 200 || !kind.starts_with("image/") {
        return Err("Slack did not hand it over (the app may need the newest manifest from the Channels screen, then reinstalling)".into());
    }
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(MAX_PICTURE + 1)
        .read_to_vec()
        .map_err(|e| scrub(&e))?;
    if bytes.len() as u64 > MAX_PICTURE {
        return Err(format!(
            "it is bigger than {} MB",
            MAX_PICTURE / (1024 * 1024)
        ));
    }
    Ok(bytes)
}

/// Sends file `name` with its `bytes` to `to` (a person, whose private chat is opened, or a
/// chat), with `caption` above it: Slack's upload in three steps, an address to send it to, the
/// bytes, then the file shared.
pub fn upload(
    base: &str,
    bot: &Secret,
    to: &Place,
    name: &str,
    bytes: &[u8],
    caption: Option<&str>,
) -> Result<(), Error> {
    let agent = agent(Duration::from_secs(300));
    // A person's id names their private chat to send messages, but files need the chat's own id.
    let channel = if to.chat.starts_with('U') || to.chat.starts_with('W') {
        let v = call(
            &agent,
            base,
            bot,
            "conversations.open",
            &[("users", to.chat.clone())],
        )?;
        v["channel"]["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| Error::Network("Slack opened no private chat".into()))?
    } else {
        to.chat.clone()
    };
    let v = call(
        &agent,
        base,
        bot,
        "files.getUploadURLExternal",
        &[
            ("filename", name.to_owned()),
            ("length", bytes.len().to_string()),
        ],
    )?;
    let (Some(url), Some(id)) = (v["upload_url"].as_str(), v["file_id"].as_str()) else {
        return Err(Error::Network("Slack gave nowhere to send the file".into()));
    };
    let resp = agent
        .post(url)
        .header("Content-Type", "application/octet-stream")
        .send(bytes)
        .map_err(|e| Error::Network(e.to_string().replace(bot.as_str(), "…")))?;
    if resp.status().as_u16() != 200 {
        return Err(Error::Network(format!(
            "Slack answered {} to the file",
            resp.status().as_u16()
        )));
    }
    let mut params = vec![
        ("files", json!([{"id": id, "title": name}]).to_string()),
        ("channel_id", channel),
    ];
    if let Some(thread) = &to.thread {
        params.push(("thread_ts", thread.clone()));
    }
    if let Some(caption) = caption.filter(|c| !c.trim().is_empty()) {
        params.push(("initial_comment", caption.to_owned()));
    }
    call(&agent, base, bot, "files.completeUploadExternal", &params).map(drop)
}

/// Takes the buttons off message `ts` in `channel`, keeping its words (`blocks` as it was).
pub(super) fn unbutton(
    agent: &ureq::Agent,
    base: &str,
    bot: &Secret,
    channel: &str,
    ts: &str,
    text: &str,
    blocks: &Value,
) {
    let kept: Vec<Value> = blocks
        .as_array()
        .map(|b| {
            b.iter()
                .filter(|b| b.get("type").and_then(Value::as_str) != Some("actions"))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let _ = call(
        agent,
        base,
        bot,
        "chat.update",
        &[
            ("channel", channel.to_owned()),
            ("ts", ts.to_owned()),
            ("text", text.to_owned()),
            ("blocks", Value::Array(kept).to_string()),
        ],
    );
}

/// Sends one thing, waiting out Slack's rate limit once.
pub(super) fn send(agent: &ureq::Agent, base: &str, bot: &Secret, out: &Out) -> Result<(), Error> {
    let attempt = || send_once(agent, base, bot, out);
    match attempt() {
        Err(Error::RetryAfter(s)) => {
            thread::sleep(Duration::from_secs(s.min(60)));
            attempt()
        }
        other => other,
    }
}

fn send_once(agent: &ureq::Agent, base: &str, bot: &Secret, out: &Out) -> Result<(), Error> {
    match out {
        // Slack has no "typing…" for bots; the mark on the message stands in.
        Out::Typing { .. } => Ok(()),
        // Buttons come off as soon as one is pressed (see `listen`), with the message's words. An
        // app keeps no session to sign out of.
        Out::Unbutton { .. } | Out::Unlink => Ok(()),
        Out::Working { to, message, on } => {
            let method = if *on {
                "reactions.add"
            } else {
                "reactions.remove"
            };
            let params = [
                ("channel", to.chat.clone()),
                ("timestamp", message.clone()),
                ("name", WORKING.to_owned()),
            ];
            match call(agent, base, bot, method, &params) {
                Err(Error::Refused(why)) if why == "already_reacted" || why == "no_reaction" => {
                    Ok(())
                }
                other => other.map(drop),
            }
        }
        // Slack has no note on a press; the callback names who pressed, and a note goes to them.
        Out::Answered { callback, note } => match note {
            Some(note) => post(
                agent,
                base,
                bot,
                &Place::private(callback),
                &format!("_{note}_"),
                &[],
            ),
            None => Ok(()),
        },
        Out::Say { to, text, buttons } => post(agent, base, bot, to, text, buttons),
    }
}

/// Posts `markdown` to `to`, as many messages as it takes, buttons under the last.
fn post(
    agent: &ureq::Agent,
    base: &str,
    bot: &Secret,
    to: &Place,
    markdown: &str,
    buttons: &[Button],
) -> Result<(), Error> {
    let messages = format::render(markdown);
    let last = messages.len().saturating_sub(1);
    for (i, sections) in messages.iter().enumerate() {
        let mut blocks: Vec<Value> = sections
            .iter()
            .map(|s| json!({"type": "section", "text": {"type": "mrkdwn", "text": s}}))
            .collect();
        if i == last {
            blocks.extend(button_rows(buttons));
        }
        let fallback: String = sections
            .first()
            .map(|s| s.chars().take(300).collect())
            .unwrap_or_default();
        let mut params = vec![
            ("channel", to.chat.clone()),
            ("text", fallback),
            ("blocks", Value::Array(blocks).to_string()),
            ("unfurl_links", "false".to_owned()),
        ];
        if let Some(thread) = &to.thread {
            params.push(("thread_ts", thread.clone()));
        }
        call(agent, base, bot, "chat.postMessage", &params)?;
    }
    Ok(())
}

/// Buttons as rows of Block Kit buttons, each sending its data back as its value.
fn button_rows(buttons: &[Button]) -> Vec<Value> {
    buttons
        .chunks(ROW)
        .enumerate()
        .map(|(row, chunk)| {
            let elements: Vec<Value> = chunk
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let label: String = b.label.chars().take(LABEL).collect();
                    json!({
                        "type": "button",
                        "text": {"type": "plain_text", "text": label},
                        "value": b.data,
                        "action_id": format!("b{row}-{i}"),
                    })
                })
                .collect();
            json!({"type": "actions", "elements": elements})
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn tokens() -> Tokens {
        Tokens::parse("xapp-1-A0-123-abcdef", "xoxb-12-34-abcdefgh").unwrap()
    }

    #[test]
    fn only_tokens_shaped_like_slacks_are_taken_and_they_round_trip_sealed() {
        assert!(
            Tokens::parse("xoxb-12-34-abcdefgh", "xapp-1-A0-123-abcdef").is_none(),
            "swapped"
        );
        assert!(Tokens::parse("xapp-1-A0-123-abcdef", "xoxb-12 34-abcdefgh").is_none());
        assert!(
            Tokens::parse("xapp-1", "xoxb-12-34-abcdefgh").is_none(),
            "too short"
        );
        let t = tokens();
        let back = Tokens::from_secret(&t.secret()).unwrap();
        assert_eq!(back.bot.as_str(), "xoxb-12-34-abcdefgh");
        assert!(
            !format!("{t:?}").contains("xoxb-12"),
            "kept out of debug output"
        );
    }

    #[test]
    fn slacks_answers_are_read_into_words_that_never_hold_the_token() {
        let t = tokens();
        let (base, seen) = crate::testing::serve(vec![
            (200, r#"{"ok":false,"error":"invalid_auth"}"#),
            (200, r#"{"ok":false,"error":"channel_not_found"}"#),
            (
                200,
                r#"{"ok":true,"user_id":"UBOT","user":"oido","team":"Acme"}"#,
            ),
        ]);
        assert_eq!(me(&base, &t.bot), Err(Error::Unauthorized));
        assert_eq!(
            me(&base, &t.bot),
            Err(Error::Refused("channel_not_found".into()))
        );
        assert_eq!(
            me(&base, &t.bot).unwrap(),
            Me {
                user_id: "UBOT".into(),
                name: "oido".into(),
                team: "Acme".into()
            }
        );
        let req = seen.recv().unwrap();
        assert!(
            req.starts_with("post /auth.test ") && req.contains("authorization: bearer xoxb-12")
        );
        let gone = me("http://127.0.0.1:1", &t.bot).unwrap_err().to_string();
        assert!(!gone.contains("xoxb"), "{gone}");
    }

    #[test]
    fn a_message_goes_as_sections_in_its_thread_with_buttons_under_it() {
        let t = tokens();
        let (base, seen) = crate::testing::serve(vec![(200, r#"{"ok":true}"#)]);
        let out = Out::Say {
            to: Place {
                chat: "C1".into(),
                thread: Some("17.5".into()),
            },
            text: "**Hi** there".into(),
            buttons: vec![Button {
                label: "Yes".into(),
                data: "b:1:0".into(),
            }],
        };
        send(&agent(Duration::from_secs(5)), &base, &t.bot, &out).unwrap();
        let req = seen.recv().unwrap();
        let body = percent_encoding::percent_decode_str(req.split("\r\n\r\n").nth(1).unwrap())
            .decode_utf8()
            .unwrap()
            .into_owned();
        assert!(req.starts_with("post /chat.postmessage "), "{req}");
        assert!(
            body.contains("channel=c1") && body.contains("thread_ts=17.5"),
            "{body}"
        );
        assert!(body.contains(r#""text":"*hi* there""#), "mrkdwn: {body}");
        assert!(body.contains(r#""value":"b:1:0""#) && body.contains(r#""type":"actions""#));
    }

    #[test]
    fn a_working_mark_goes_on_and_comes_off_as_a_reaction() {
        let t = tokens();
        let (base, seen) = crate::testing::serve(vec![
            (200, r#"{"ok":true}"#),
            (200, r#"{"ok":false,"error":"no_reaction"}"#),
        ]);
        let mark = |on| Out::Working {
            to: Place {
                chat: "D1".into(),
                thread: None,
            },
            message: "17.5".into(),
            on,
        };
        let a = agent(Duration::from_secs(5));
        send(&a, &base, &t.bot, &mark(true)).unwrap();
        send(&a, &base, &t.bot, &mark(false)).unwrap();
        assert!(seen.recv().unwrap().starts_with("post /reactions.add "));
        assert!(seen.recv().unwrap().starts_with("post /reactions.remove "));
    }

    #[test]
    fn a_shared_file_is_fetched_only_from_slacks_file_host() {
        let t = tokens();
        let err = download(
            &agent(Duration::from_secs(5)),
            &t.bot,
            "http://127.0.0.1:1/x.png",
        )
        .unwrap_err();
        assert!(err.contains("not on Slack's file host"), "{err}");
    }

    #[test]
    fn a_file_goes_up_in_three_steps_to_the_persons_private_chat() {
        let t = tokens();
        let (base, seen) = crate::testing::serve(vec![(200, "")]);
        // The address the file goes to is the test server too, so it is known only now.
        let up: &'static str = Box::leak(
            format!(r#"{{"ok":true,"upload_url":"{base}/upload","file_id":"F1"}}"#)
                .into_boxed_str(),
        );
        let (api, calls) = crate::testing::serve(vec![
            (200, r#"{"ok":true,"channel":{"id":"D9"}}"#),
            (200, up),
            (200, r#"{"ok":true}"#),
        ]);
        upload(
            &api,
            &t.bot,
            &Place::private("U1"),
            "report.txt",
            b"numbers",
            Some("Here"),
        )
        .unwrap();
        let decode = |req: String| {
            percent_encoding::percent_decode_str(&req)
                .decode_utf8()
                .unwrap()
                .into_owned()
        };
        assert!(decode(calls.recv().unwrap()).contains("users=u1"));
        let asked = decode(calls.recv().unwrap());
        assert!(
            asked.starts_with("post /files.getuploadurlexternal ") && asked.contains("length=7"),
            "{asked}"
        );
        let sent = seen.recv().unwrap();
        assert!(
            sent.starts_with("post /upload ") && sent.ends_with("numbers"),
            "{sent}"
        );
        assert!(
            !sent.contains("xoxb"),
            "the token does not go with the bytes"
        );
        let shared = decode(calls.recv().unwrap());
        assert!(
            shared.contains("channel_id=d9") && shared.contains(r#""id":"f1""#),
            "{shared}"
        );
        assert!(shared.contains("initial_comment=here"));
    }
}

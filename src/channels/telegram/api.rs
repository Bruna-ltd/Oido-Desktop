//! Telegram's Bot API (<https://core.telegram.org/bots/api>): what it sends, and the calls the
//! app makes. The token is in every address, so it never appears in an error: errors are worded
//! here.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::format;
use crate::channels::{Button, Out, Place};
use crate::update::VERSION;

pub const API: &str = "https://api.telegram.org";
/// How long one poll waits on Telegram for something to arrive.
pub(super) const POLL_SECS: u64 = 50;
/// The longest a message may be, in characters (UTF-16 units, strictly; see `format::split`).
pub const MAX_TEXT: usize = 4096;
/// The biggest picture fetched from someone, in bytes.
pub const MAX_PICTURE: u64 = 10 * 1024 * 1024;
/// The biggest file sent as a photo (Telegram's limit); bigger pictures go as files.
const MAX_PHOTO: usize = 10 * 1024 * 1024;
/// The biggest file a bot can send (Telegram's limit).
pub const MAX_FILE: u64 = 50 * 1024 * 1024;

/// A bot's token. Checked for shape before it is put in an address.
#[derive(Clone)]
pub struct Token(Arc<Zeroizing<String>>);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(…)")
    }
}

impl Token {
    /// A token as BotFather gives it: the bot's number, a colon, then letters, digits, `_` and `-`.
    pub fn parse(raw: &str) -> Option<Token> {
        let raw = raw.trim();
        let (id, secret) = raw.split_once(':')?;
        let ok = !id.is_empty()
            && id.bytes().all(|b| b.is_ascii_digit())
            && secret.len() >= 20
            && secret
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        ok.then(|| Token(Arc::new(Zeroizing::new(raw.to_owned()))))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Why a call did not go through, in words safe to show.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// Telegram does not know the token (revoked, or mistyped).
    Unauthorized,
    /// Another program is reading this bot's updates, or it has a webhook.
    Conflict,
    /// Too many calls: wait this many seconds.
    RetryAfter(u64),
    /// Telegram refused the call, with its reason.
    Refused(String),
    /// Telegram could not be reached, or its answer could not be read.
    Network(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized => f.write_str("Telegram does not accept this token"),
            Self::Conflict => f.write_str(
                "another program is reading this bot's messages (or it has a webhook set)",
            ),
            Self::RetryAfter(s) => write!(f, "Telegram asks to wait {s} seconds"),
            Self::Refused(why) => write!(f, "Telegram refused: {why}"),
            Self::Network(why) => write!(f, "cannot reach Telegram: {why}"),
        }
    }
}

// ---- what Telegram sends

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Update {
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<Message>,
    #[serde(default)]
    pub callback_query: Option<Callback>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Message {
    pub message_id: i64,
    /// When it was sent, in seconds since the epoch.
    pub date: i64,
    pub chat: Chat,
    #[serde(default)]
    pub from: Option<User>,
    #[serde(default)]
    pub text: Option<String>,
    /// A picture's caption, or a file's.
    #[serde(default)]
    pub caption: Option<String>,
    /// A photo, in the sizes Telegram keeps, smallest first.
    #[serde(default)]
    pub photo: Vec<PhotoSize>,
    /// A file sent as one (a picture sent uncompressed is one too).
    #[serde(default)]
    pub document: Option<Document>,
    /// The picture's bytes, fetched before the app hears the message, or why they could not be.
    /// Only for people paired; never from Telegram's JSON.
    #[serde(skip)]
    pub picture: Option<Result<Vec<u8>, String>>,
}

impl Message {
    /// The picture in it, if any: the biggest size of a photo, or a file that is an image.
    pub fn picture_file(&self) -> Option<(&str, Option<u64>)> {
        if let Some(p) = self.photo.last() {
            return Some((&p.file_id, p.file_size));
        }
        self.document
            .as_ref()
            .filter(|d| {
                d.mime_type
                    .as_deref()
                    .is_some_and(|m| m.starts_with("image/"))
            })
            .map(|d| (d.file_id.as_str(), d.file_size))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PhotoSize {
    pub file_id: String,
    #[serde(default)]
    pub file_size: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Document {
    pub file_id: String,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub file_size: Option<u64>,
}

/// Where Telegram keeps a file for a while, to fetch it from.
#[derive(Deserialize)]
struct FileRef {
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    file_size: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Chat {
    pub id: i64,
    /// `private`, `group`, `supergroup` or `channel`.
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct User {
    pub id: i64,
    #[serde(default)]
    pub is_bot: bool,
    pub first_name: String,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

impl User {
    /// The name to show: first and last.
    pub fn name(&self) -> String {
        match &self.last_name {
            Some(last) if !last.trim().is_empty() => format!("{} {}", self.first_name, last),
            _ => self.first_name.clone(),
        }
    }
}

/// A press on a button under one of the bot's messages.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Callback {
    pub id: String,
    pub from: User,
    #[serde(default)]
    pub message: Option<Message>,
    #[serde(default)]
    pub data: Option<String>,
}

// ---- calls

pub(super) fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(format!("oido-desktop/{VERSION}"))
        .http_status_as_error(false)
        .build()
        .into()
}

/// Telegram's answer to every call.
#[derive(Deserialize)]
struct Answer<T> {
    ok: bool,
    result: Option<T>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    error_code: Option<u16>,
    #[serde(default)]
    parameters: Option<Parameters>,
}

#[derive(Deserialize)]
struct Parameters {
    #[serde(default)]
    retry_after: Option<u64>,
}

/// Calls `method` with a JSON `body` and reads its result.
fn call<T: DeserializeOwned>(
    agent: &ureq::Agent,
    base: &str,
    token: &Token,
    method: &str,
    body: &Value,
) -> Result<T, Error> {
    request(
        agent,
        base,
        token,
        method,
        "application/json",
        body.to_string().as_bytes(),
    )
}

/// The token's words taken out of an error, though ureq's do not carry the address.
fn scrub(token: &Token, e: &dyn std::fmt::Display) -> String {
    e.to_string().replace(token.as_str(), "…")
}

/// Calls `method` with `body` of type `kind` and reads its result.
fn request<T: DeserializeOwned>(
    agent: &ureq::Agent,
    base: &str,
    token: &Token,
    method: &str,
    kind: &str,
    body: &[u8],
) -> Result<T, Error> {
    let url = format!("{base}/bot{}/{method}", token.as_str());
    let mut resp = agent
        .post(&url)
        .header("Content-Type", kind)
        .send(body)
        .map_err(|e| Error::Network(scrub(token, &e)))?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| Error::Network(scrub(token, &e)))?;
    let answer: Answer<T> = serde_json::from_str(&text)
        .map_err(|_| Error::Network(format!("Telegram answered {status} with something else")))?;
    match (answer.ok, answer.result) {
        (true, Some(result)) => Ok(result),
        _ => Err(match answer.error_code.unwrap_or(status) {
            401 | 404 => Error::Unauthorized,
            409 => Error::Conflict,
            429 => Error::RetryAfter(answer.parameters.and_then(|p| p.retry_after).unwrap_or(5)),
            _ => Error::Refused(answer.description.unwrap_or_else(|| format!("{status}"))),
        }),
    }
}

/// Fetches file `id` someone sent, refusing one over [`MAX_PICTURE`].
pub fn download(base: &str, token: &Token, id: &str) -> Result<Vec<u8>, String> {
    let agent = agent(Duration::from_secs(60));
    let file: FileRef =
        call(&agent, base, token, "getFile", &json!({"file_id": id})).map_err(|e| e.to_string())?;
    if file.file_size.is_some_and(|n| n > MAX_PICTURE) {
        return Err(too_big());
    }
    let path = file.file_path.ok_or("Telegram has no copy of it")?;
    let url = format!("{base}/file/bot{}/{path}", token.as_str());
    let mut resp = agent.get(&url).call().map_err(|e| scrub(token, &e))?;
    if resp.status().as_u16() != 200 {
        return Err(format!("Telegram answered {}", resp.status().as_u16()));
    }
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(MAX_PICTURE + 1)
        .read_to_vec()
        .map_err(|e| scrub(token, &e))?;
    if bytes.len() as u64 > MAX_PICTURE {
        return Err(too_big());
    }
    Ok(bytes)
}

pub(super) fn too_big() -> String {
    format!("it is bigger than {} MB", MAX_PICTURE / (1024 * 1024))
}

/// Whether a file named `name` is sent as a photo: a picture Telegram shows in the chat.
fn is_photo(name: &str) -> bool {
    let lower = name.to_lowercase();
    [".png", ".jpg", ".jpeg", ".webp"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// Sends file `name` with its `bytes` to `to`, as a photo if it is a picture small enough
/// (as a file if Telegram will not take it as one), else as a file.
pub fn upload(
    base: &str,
    token: &Token,
    to: &Place,
    name: &str,
    bytes: &[u8],
    caption: Option<&str>,
) -> Result<(), Error> {
    let agent = agent(Duration::from_secs(300));
    if is_photo(name) && bytes.len() <= MAX_PHOTO {
        match send_part(
            &agent,
            base,
            token,
            ("sendPhoto", "photo"),
            &to.chat,
            name,
            bytes,
            caption,
        ) {
            Err(Error::Refused(_)) => {}
            other => return other,
        }
    }
    send_part(
        &agent,
        base,
        token,
        ("sendDocument", "document"),
        &to.chat,
        name,
        bytes,
        caption,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "one multipart call's parts, from `upload` only"
)]
fn send_part(
    agent: &ureq::Agent,
    base: &str,
    token: &Token,
    (method, field): (&str, &str),
    chat: &str,
    name: &str,
    bytes: &[u8],
    caption: Option<&str>,
) -> Result<(), Error> {
    let boundary = format!("oido-{}", uuid::Uuid::new_v4().simple());
    let mut body = Vec::with_capacity(bytes.len() + 512);
    let mut text = |key: &str, value: &str| {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{key}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    };
    text("chat_id", chat);
    if let Some(caption) = caption.filter(|c| !c.trim().is_empty()) {
        // Telegram takes captions up to 1024 characters.
        text("caption", &caption.chars().take(1024).collect::<String>());
    }
    // A file name cannot break out of its quotes or its line.
    let safe: String = name
        .chars()
        .map(|c| {
            if c == '"' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{safe}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let kind = format!("multipart/form-data; boundary={boundary}");
    request::<Value>(agent, base, token, method, &kind, &body).map(drop)
}

/// The bot behind `token`, which proves the token works.
pub fn me(base: &str, token: &Token) -> Result<User, Error> {
    call(
        &agent(Duration::from_secs(15)),
        base,
        token,
        "getMe",
        &json!({}),
    )
}

/// Puts the bot's commands in Telegram's `/` menu. Best effort.
pub fn set_commands(base: &str, token: &Token) {
    let commands: Vec<Value> = crate::channels::COMMANDS
        .iter()
        .map(|(command, description)| json!({"command": command, "description": description}))
        .collect();
    let _ = call::<bool>(
        &agent(Duration::from_secs(15)),
        base,
        token,
        "setMyCommands",
        &json!({ "commands": commands }),
    );
}

pub(super) fn updates(
    agent: &ureq::Agent,
    base: &str,
    token: &Token,
    offset: Option<i64>,
) -> Result<Vec<Update>, Error> {
    let mut body = json!({
        "timeout": POLL_SECS,
        "allowed_updates": ["message", "callback_query"],
    });
    if let Some(offset) = offset {
        body["offset"] = json!(offset);
    }
    call(agent, base, token, "getUpdates", &body)
}

/// Sends one thing, waiting out Telegram's rate limit once.
pub(super) fn send(agent: &ureq::Agent, base: &str, token: &Token, out: &Out) -> Result<(), Error> {
    let attempt = || send_once(agent, base, token, out);
    match attempt() {
        Err(Error::RetryAfter(s)) => {
            thread::sleep(Duration::from_secs(s.min(60)));
            attempt()
        }
        other => other,
    }
}

/// A chat or message id as Telegram takes it: a number where it is one.
fn id(s: &str) -> Value {
    s.parse::<i64>().map_or_else(|_| json!(s), |n| json!(n))
}

fn send_once(agent: &ureq::Agent, base: &str, token: &Token, out: &Out) -> Result<(), Error> {
    match out {
        // Telegram has no mark for a message being worked on; "typing…" stands in. A bot keeps no
        // session to sign out of.
        Out::Working { on: false, .. } | Out::Unlink => Ok(()),
        Out::Typing { to } | Out::Working { to, on: true, .. } => call::<bool>(
            agent,
            base,
            token,
            "sendChatAction",
            &json!({"chat_id": id(&to.chat), "action": "typing"}),
        )
        .map(drop),
        Out::Unbutton { to, message } => call::<Value>(
            agent,
            base,
            token,
            "editMessageReplyMarkup",
            &json!({"chat_id": id(&to.chat), "message_id": id(message), "reply_markup": {"inline_keyboard": []}}),
        )
        .map(drop),
        Out::Answered { callback, note } => {
            let mut body = json!({"callback_query_id": callback});
            if let Some(note) = note {
                body["text"] = json!(note);
            }
            call::<bool>(agent, base, token, "answerCallbackQuery", &body).map(drop)
        }
        Out::Say { to, text, buttons } => {
            let parts = format::render(text);
            let last = parts.html.len().saturating_sub(1);
            for (i, part) in parts.html.iter().enumerate() {
                let mut body = json!({
                    "chat_id": id(&to.chat),
                    "text": part,
                    "parse_mode": "HTML",
                    "link_preview_options": {"is_disabled": true},
                });
                if i == last && !buttons.is_empty() {
                    body["reply_markup"] = keyboard(buttons);
                }
                match call::<Value>(agent, base, token, "sendMessage", &body) {
                    // Formatting Telegram cannot read: the same words, unformatted.
                    Err(Error::Refused(why)) if why.contains("parse") => {
                        if let Some(text) = parts.plain.get(i) {
                            body["text"] = json!(text);
                            if let Some(map) = body.as_object_mut() {
                                map.remove("parse_mode");
                            }
                            call::<Value>(agent, base, token, "sendMessage", &body)?;
                        }
                    }
                    other => {
                        other?;
                    }
                }
            }
            Ok(())
        }
    }
}

/// Buttons, one per row.
fn keyboard(buttons: &[Button]) -> Value {
    let rows: Vec<Value> = buttons
        .iter()
        .map(|b| json!([{"text": b.label, "callback_data": b.data}]))
        .collect();
    json!({ "inline_keyboard": rows })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn only_a_token_shaped_like_botfathers_is_taken() {
        assert!(Token::parse(" 123456:AAH-abcdefghijklmnopqrstu_vw ").is_some());
        for bad in [
            "",
            "123456",
            ":AAHabcdefghijklmnopqrstuvw",
            "12a:AAHabcdefghijklmnopqrstuvw",
            "123:short",
            "123:AAHabcdefghij/../klmnopqrstuvw",
            "123:AAH abcdefghijklmnopqrstuvw",
        ] {
            assert!(Token::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn telegrams_errors_are_read_into_words_that_never_hold_the_token() {
        let token = Token::parse("123:AAHabcdefghijklmnopqrstuvw").unwrap();
        let (base, _seen) = crate::testing::serve(vec![
            (
                401,
                r#"{"ok":false,"error_code":401,"description":"Unauthorized"}"#,
            ),
            (
                409,
                r#"{"ok":false,"error_code":409,"description":"Conflict: terminated by other getUpdates request"}"#,
            ),
            (
                429,
                r#"{"ok":false,"error_code":429,"description":"Too Many Requests","parameters":{"retry_after":7}}"#,
            ),
            (
                200,
                r#"{"ok":true,"result":{"id":1,"is_bot":true,"first_name":"Oido","username":"oido_bot"}}"#,
            ),
        ]);
        assert_eq!(me(&base, &token), Err(Error::Unauthorized));
        assert_eq!(me(&base, &token), Err(Error::Conflict));
        assert_eq!(me(&base, &token), Err(Error::RetryAfter(7)));
        assert_eq!(
            me(&base, &token).unwrap().username.as_deref(),
            Some("oido_bot")
        );
        let gone = me("http://127.0.0.1:1", &token).unwrap_err().to_string();
        assert!(!gone.contains(token.as_str()), "{gone}");
    }

    #[test]
    fn a_message_telegram_cannot_parse_goes_again_unformatted_with_its_buttons() {
        let token = Token::parse("123:AAHabcdefghijklmnopqrstuvw").unwrap();
        let (base, seen) = crate::testing::serve(vec![
            (
                400,
                r#"{"ok":false,"error_code":400,"description":"Bad Request: can't parse entities"}"#,
            ),
            (200, r#"{"ok":true,"result":{}}"#),
        ]);
        let out = Out::Say {
            to: Place::private("42"),
            text: "hi".into(),
            buttons: vec![Button {
                label: "Yes".into(),
                data: "a:1".into(),
            }],
        };
        send(&agent(Duration::from_secs(5)), &base, &token, &out).unwrap();
        let first = seen.recv().unwrap();
        let second = seen.recv().unwrap();
        assert!(first.contains("\"parse_mode\":\"html\""), "{first}");
        assert!(
            !second.contains("parse_mode") && second.contains("\"text\":\"hi\""),
            "{second}"
        );
        assert!(second.contains("inline_keyboard"), "{second}");
    }

    #[test]
    fn updates_read_messages_and_button_presses() {
        let raw = r#"[
            {"update_id":5,"message":{"message_id":1,"date":100,"chat":{"id":42,"type":"private"},
             "from":{"id":42,"is_bot":false,"first_name":"Ada","last_name":"L","username":"ada"},"text":"/start"}},
            {"update_id":6,"callback_query":{"id":"cb","from":{"id":42,"first_name":"Ada"},"data":"agent:x"}},
            {"update_id":7,"edited_message":{"message_id":1}}
        ]"#;
        let list: Vec<Update> = serde_json::from_str(raw).unwrap();
        assert_eq!(list.len(), 3);
        let m = list[0].message.as_ref().unwrap();
        assert_eq!(
            (m.chat.kind.as_str(), m.from.as_ref().unwrap().name()),
            ("private", "Ada L".to_owned())
        );
        assert_eq!(
            list[1].callback_query.as_ref().unwrap().data.as_deref(),
            Some("agent:x")
        );
        assert!(list[2].message.is_none() && list[2].callback_query.is_none());
    }

    #[test]
    fn a_file_goes_as_a_photo_or_else_as_a_document_with_a_safe_name() {
        let token = Token::parse("123:AAHabcdefghijklmnopqrstuvw").unwrap();
        let (base, seen) = crate::testing::serve(vec![
            (
                400,
                r#"{"ok":false,"error_code":400,"description":"Bad Request: IMAGE_PROCESS_FAILED"}"#,
            ),
            (200, r#"{"ok":true,"result":{}}"#),
            (200, r#"{"ok":true,"result":{}}"#),
        ]);
        upload(
            &base,
            &token,
            &Place::private("42"),
            "chart\"x.png",
            b"not really",
            Some("Here"),
        )
        .unwrap();
        let photo = seen.recv().unwrap();
        assert!(
            photo.contains("/sendphoto ") && photo.contains("name=\"photo\""),
            "{photo}"
        );
        let doc = seen.recv().unwrap();
        assert!(doc.contains("/senddocument "), "{doc}");
        assert!(doc.contains("filename=\"chart_x.png\"") && doc.contains("not really"));
        assert!(doc.contains("name=\"caption\"\r\n\r\nhere"));
        upload(
            &base,
            &token,
            &Place::private("42"),
            "notes.txt",
            b"hi",
            None,
        )
        .unwrap();
        let txt = seen.recv().unwrap();
        assert!(
            txt.contains("/senddocument ") && !txt.contains("caption"),
            "{txt}"
        );
    }
}

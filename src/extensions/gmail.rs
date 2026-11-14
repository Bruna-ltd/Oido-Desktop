//! Gmail: search and read mail, and see the labels. Read only; writing comes later.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};

use super::google::{Api, Failure};
use super::mime::Outgoing;
use super::{Access, Extension, ToolDef};
use crate::mcp::catalog::Logo;

/// Read, labels, drafts and the trash (not deleting for good).
const MODIFY: &str = "https://www.googleapis.com/auth/gmail.modify";
const SEND: &str = "https://www.googleapis.com/auth/gmail.send";
const BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const DEFAULT_RESULTS: u64 = 10;
const MAX_RESULTS: u64 = 25;

pub struct Gmail;

impl Extension for Gmail {
    fn title(&self) -> &'static str {
        "Gmail"
    }

    fn logo(&self) -> Logo {
        super::marks::GMAIL
    }

    fn blurb(&self) -> &'static str {
        "Search and read the person's mail, write drafts, send and reply, and sort it with labels."
    }

    fn scopes(&self) -> &'static [&'static str] {
        &[MODIFY, SEND]
    }

    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef {
                name: "search",
                description: "Search the person's Gmail with Gmail's own search syntax (from:, to:, subject:, after:2025/01/01, is:unread, has:attachment, label:...). Gives one line per message: its id, date, sender, subject and a snippet. Read one with read_message.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "What to search for. Empty lists the newest mail."},
                        "max_results": {"type": "integer", "description": "How many messages, 1 to 25. Default 10."},
                        "page_token": {"type": "string", "description": "From the end of an earlier search, for the next page."}
                    }
                }),
            },
            ToolDef {
                name: "read_message",
                description: "Read one email: its headers, its text and the names of its attachments.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {"id": {"type": "string", "description": "The message id from search."}},
                    "required": ["id"]
                }),
            },
            ToolDef {
                name: "read_thread",
                description: "Read every email of a conversation, oldest first.",
                access: Access::Reads,
                schema: json!({
                    "type": "object",
                    "properties": {"id": {"type": "string", "description": "The thread id, shown by read_message."}},
                    "required": ["id"]
                }),
            },
            ToolDef {
                name: "list_labels",
                description: "List the person's Gmail labels: the built-in ones and their own.",
                access: Access::Reads,
                schema: json!({"type": "object", "properties": {}}),
            },
            ToolDef {
                name: "create_draft",
                description: "Save a draft email in the person's Gmail without sending it. Plain text, plain addresses (name@example.com).",
                access: Access::Changes,
                schema: compose_schema(true),
            },
            ToolDef {
                name: "send_message",
                description: "Send an email from the person's Gmail at once. It cannot be unsent. Plain text, plain addresses (name@example.com). Prefer create_draft unless the person asked you to send.",
                access: Access::Changes,
                schema: compose_schema(true),
            },
            ToolDef {
                name: "send_draft",
                description: "Send a draft that is already saved, as it is now. It cannot be unsent.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {"draft_id": {"type": "string", "description": "The draft's id, from create_draft."}},
                    "required": ["draft_id"]
                }),
            },
            ToolDef {
                name: "reply",
                description: "Reply to an email in its conversation, with the subject and threading filled in. It goes to whoever wrote to the person (or their Reply-To address); reply_all also keeps everyone else on it. Sends at once unless as_draft is true.",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "message_id": {"type": "string", "description": "The message to answer, from search."},
                        "body": {"type": "string", "description": "What to say, as plain text. The quoted original is not added."},
                        "reply_all": {"type": "boolean", "description": "Also keep everyone else who was on it. Default false."},
                        "as_draft": {"type": "boolean", "description": "Save a draft instead of sending. Default false."}
                    },
                    "required": ["message_id", "body"]
                }),
            },
            ToolDef {
                name: "modify_labels",
                description: "Add and remove labels on a message (ids from list_labels, like INBOX, UNREAD, STARRED or Label_12).",
                access: Access::Changes,
                schema: json!({
                    "type": "object",
                    "properties": {
                        "id": {"type": "string", "description": "The message id."},
                        "add": {"type": "array", "items": {"type": "string"}, "description": "Label ids to add."},
                        "remove": {"type": "array", "items": {"type": "string"}, "description": "Label ids to remove."}
                    },
                    "required": ["id"]
                }),
            },
            ToolDef {
                name: "archive",
                description: "Take a message out of the inbox. It stays in All Mail.",
                access: Access::Changes,
                schema: id_schema(),
            },
            ToolDef {
                name: "mark_read",
                description: "Mark a message as read.",
                access: Access::Changes,
                schema: id_schema(),
            },
            ToolDef {
                name: "trash",
                description: "Move a message to the trash, where Gmail keeps it for 30 days.",
                access: Access::Changes,
                schema: id_schema(),
            },
        ]
    }

    fn call(&self, tool: &str, args: &Value, api: &Api) -> Result<String, Failure> {
        match tool {
            "search" => search(args, api),
            "read_message" => {
                let msg = api.get(
                    &format!("{BASE}/messages/{}", id_arg(args)?),
                    &[("format", "full")],
                )?;
                Ok(render_message(&msg))
            }
            "read_thread" => {
                let thread = api.get(
                    &format!("{BASE}/threads/{}", id_arg(args)?),
                    &[("format", "full")],
                )?;
                let messages = thread["messages"].as_array().map_or(&[][..], Vec::as_slice);
                if messages.is_empty() {
                    return Ok("The thread has no messages.".to_owned());
                }
                Ok(messages
                    .iter()
                    .map(render_message)
                    .collect::<Vec<_>>()
                    .join("\n\n---\n\n"))
            }
            "list_labels" => {
                let labels = api.get(&format!("{BASE}/labels"), &[])?;
                Ok(render_labels(&labels))
            }
            "create_draft" => {
                let mail = compose(args)?;
                let made = api.post(
                    &format!("{BASE}/drafts"),
                    &[],
                    &json!({"message": message_body(&mail, args)?}),
                )?;
                Ok(format!(
                    "Draft saved. Draft id {} (message {}).",
                    made["id"].as_str().unwrap_or("?"),
                    made["message"]["id"].as_str().unwrap_or("?")
                ))
            }
            "send_message" => {
                let mail = compose(args)?;
                let sent = api.post(
                    &format!("{BASE}/messages/send"),
                    &[],
                    &message_body(&mail, args)?,
                )?;
                Ok(sent_line(&sent))
            }
            "send_draft" => {
                let id = word(args, "draft_id")?;
                let sent = api.post(&format!("{BASE}/drafts/send"), &[], &json!({"id": id}))?;
                Ok(sent_line(&sent))
            }
            "reply" => reply(args, api),
            "modify_labels" => {
                let id = id_arg(args)?;
                let (add, remove) = (words(args, "add")?, words(args, "remove")?);
                if add.is_empty() && remove.is_empty() {
                    return Err(Failure::other("give labels to add or remove"));
                }
                modify(api, id, &add, &remove)
            }
            "archive" => modify(api, id_arg(args)?, &[], &["INBOX"]),
            "mark_read" => modify(api, id_arg(args)?, &[], &["UNREAD"]),
            "trash" => {
                let id = id_arg(args)?;
                api.post(&format!("{BASE}/messages/{id}/trash"), &[], &json!({}))?;
                Ok(format!("Message {id} moved to the trash."))
            }
            other => Err(Failure::other(format!("Gmail has no tool called {other}"))),
        }
    }
}

fn id_schema() -> Value {
    json!({
        "type": "object",
        "properties": {"id": {"type": "string", "description": "The message id from search."}},
        "required": ["id"]
    })
}

fn compose_schema(thread: bool) -> Value {
    let mut props = json!({
        "to": {"type": "array", "items": {"type": "string"}, "description": "Email addresses to send to."},
        "cc": {"type": "array", "items": {"type": "string"}},
        "bcc": {"type": "array", "items": {"type": "string"}},
        "subject": {"type": "string"},
        "body": {"type": "string", "description": "The message, as plain text."}
    });
    if thread {
        props["thread_id"] = json!({"type": "string", "description": "Put it in this conversation (a thread id from read_message). Optional."});
    }
    json!({"type": "object", "properties": props, "required": ["to", "subject", "body"]})
}

/// A word from the model that goes into an address or a label: letters, digits, `_` and `-`.
fn safe(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn word<'a>(args: &'a Value, name: &str) -> Result<&'a str, Failure> {
    match args[name].as_str() {
        Some(w) if safe(w) => Ok(w),
        Some(_) => Err(Failure::other(format!("`{name}` is not a Gmail id"))),
        None => Err(Failure::other(format!("`{name}` is required"))),
    }
}

/// A list of ids from the model, each checked.
fn words<'a>(args: &'a Value, name: &str) -> Result<Vec<&'a str>, Failure> {
    args[name]
        .as_array()
        .into_iter()
        .flatten()
        .map(|w| match w.as_str() {
            Some(w) if safe(w) => Ok(w),
            _ => Err(Failure::other(format!(
                "`{name}` holds something that is not a label id"
            ))),
        })
        .collect()
}

/// Addresses given as a list, or as one string with commas.
fn recipients(args: &Value, name: &str) -> Vec<String> {
    match &args[name] {
        Value::Array(list) => list
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::to_owned)
            .collect(),
        Value::String(s) => s
            .split(',')
            .map(|a| a.trim().to_owned())
            .filter(|a| !a.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn compose(args: &Value) -> Result<Outgoing, Failure> {
    let (Some(subject), Some(body)) = (args["subject"].as_str(), args["body"].as_str()) else {
        return Err(Failure::other("`subject` and `body` are required"));
    };
    Ok(Outgoing {
        to: recipients(args, "to"),
        cc: recipients(args, "cc"),
        bcc: recipients(args, "bcc"),
        subject: subject.to_owned(),
        body: body.to_owned(),
        ..Outgoing::default()
    })
}

/// What Gmail takes for a message: the text, and the conversation it joins when one is named.
fn message_body(mail: &Outgoing, args: &Value) -> Result<Value, Failure> {
    let mut body = json!({"raw": mail.raw()?});
    if args["thread_id"].as_str().is_some_and(|t| !t.is_empty()) {
        body["threadId"] = json!(word(args, "thread_id")?);
    }
    Ok(body)
}

fn sent_line(sent: &Value) -> String {
    format!(
        "Sent. Message id {} (thread {}).",
        sent["id"].as_str().unwrap_or("?"),
        sent["threadId"].as_str().unwrap_or("?")
    )
}

fn modify(api: &Api, id: &str, add: &[&str], remove: &[&str]) -> Result<String, Failure> {
    api.post(
        &format!("{BASE}/messages/{id}/modify"),
        &[],
        &json!({"addLabelIds": add, "removeLabelIds": remove}),
    )?;
    Ok(format!(
        "Message {id} updated (added: {}; removed: {}).",
        add.join(", "),
        remove.join(", ")
    ))
}

/// The bare addresses in a header such as `Ann <a@x.org>, "Lee, Bo" <b@y.org>, c@z.org`.
fn addresses_in(header: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in header.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            ',' if !quoted => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
        .iter()
        .filter_map(|p| {
            let p = p.trim();
            let inner = match (p.rfind('<'), p.rfind('>')) {
                (Some(a), Some(b)) if a < b => &p[a + 1..b],
                _ => p,
            };
            let inner = inner.trim();
            (inner.contains('@') && !inner.contains(char::is_whitespace)).then(|| inner.to_owned())
        })
        .collect()
}

/// A reply built from the message it answers: who to, the subject, and the threading headers.
fn reply_to(
    original: &Value,
    body: &str,
    reply_all: bool,
    own: Option<&str>,
) -> Result<(Outgoing, String), Failure> {
    let from = header(original, "Reply-To");
    let from = if from.is_empty() {
        header(original, "From")
    } else {
        from
    };
    let to = addresses_in(from);
    let Some(first) = to.first().cloned() else {
        return Err(Failure::other("the message has no sender to reply to"));
    };
    let mut cc = Vec::new();
    if reply_all {
        for a in addresses_in(header(original, "To"))
            .into_iter()
            .chain(addresses_in(header(original, "Cc")))
        {
            let lower = a.to_lowercase();
            let mine = own.is_some_and(|o| o.eq_ignore_ascii_case(&a));
            if !mine
                && !to.iter().any(|t| t.to_lowercase() == lower)
                && !cc.iter().any(|c: &String| c.to_lowercase() == lower)
            {
                cc.push(a);
            }
        }
    }
    let subject = header(original, "Subject");
    let subject = if subject.to_lowercase().starts_with("re:") {
        subject.to_owned()
    } else {
        format!("Re: {subject}")
    };
    let message_id = header(original, "Message-ID");
    let references = header(original, "References");
    let mail = Outgoing {
        to: vec![first],
        cc,
        subject,
        body: body.to_owned(),
        in_reply_to: (!message_id.is_empty()).then(|| message_id.to_owned()),
        references: (!message_id.is_empty())
            .then(|| format!("{references} {message_id}").trim().to_owned()),
        ..Outgoing::default()
    };
    Ok((
        mail,
        original["threadId"].as_str().unwrap_or_default().to_owned(),
    ))
}

fn reply(args: &Value, api: &Api) -> Result<String, Failure> {
    let id = word(args, "message_id")?;
    let Some(body) = args["body"].as_str().filter(|b| !b.trim().is_empty()) else {
        return Err(Failure::other("`body` is required"));
    };
    let original = api.get(
        &format!("{BASE}/messages/{id}"),
        &[
            ("format", "metadata"),
            ("metadataHeaders", "Message-ID"),
            ("metadataHeaders", "References"),
            ("metadataHeaders", "Reply-To"),
            ("metadataHeaders", "From"),
            ("metadataHeaders", "To"),
            ("metadataHeaders", "Cc"),
            ("metadataHeaders", "Subject"),
        ],
    )?;
    let reply_all = args["reply_all"].as_bool() == Some(true);
    let own = if reply_all {
        api.get(&format!("{BASE}/profile"), &[])?["emailAddress"]
            .as_str()
            .map(str::to_owned)
    } else {
        None
    };
    let (mail, thread) = reply_to(&original, body, reply_all, own.as_deref())?;
    let mut message = json!({"raw": mail.raw()?});
    if !thread.is_empty() {
        message["threadId"] = json!(thread);
    }
    if args["as_draft"].as_bool() == Some(true) {
        let made = api.post(&format!("{BASE}/drafts"), &[], &json!({"message": message}))?;
        Ok(format!(
            "Reply saved as a draft. Draft id {} (to {}).",
            made["id"].as_str().unwrap_or("?"),
            mail.to.join(", ")
        ))
    } else {
        let sent = api.post(&format!("{BASE}/messages/send"), &[], &message)?;
        Ok(format!("{} To: {}.", sent_line(&sent), mail.to.join(", ")))
    }
}

/// An id from the model, which goes into an address: letters, digits, `_` and `-` only.
fn id_arg(args: &Value) -> Result<&str, Failure> {
    match args["id"].as_str() {
        Some(id)
            if !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') =>
        {
            Ok(id)
        }
        Some(_) => Err(Failure::other("that is not a Gmail id")),
        None => Err(Failure::other("`id` is required")),
    }
}

fn search(args: &Value, api: &Api) -> Result<String, Failure> {
    let max = args["max_results"]
        .as_u64()
        .unwrap_or(DEFAULT_RESULTS)
        .clamp(1, MAX_RESULTS)
        .to_string();
    let mut query = vec![("maxResults", max.as_str())];
    if let Some(q) = args["query"].as_str().filter(|q| !q.trim().is_empty()) {
        query.push(("q", q));
    }
    if let Some(t) = args["page_token"].as_str().filter(|t| !t.is_empty()) {
        query.push(("pageToken", t));
    }
    let found = api.get(&format!("{BASE}/messages"), &query)?;
    let ids: Vec<&str> = found["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str())
        .collect();
    if ids.is_empty() {
        return Ok("No messages found.".to_owned());
    }
    let mut lines = Vec::new();
    for id in ids {
        let msg = api.get(
            &format!("{BASE}/messages/{id}"),
            &[
                ("format", "metadata"),
                ("metadataHeaders", "From"),
                ("metadataHeaders", "Subject"),
                ("metadataHeaders", "Date"),
            ],
        )?;
        lines.push(summary_line(&msg));
    }
    if let Some(next) = found["nextPageToken"].as_str() {
        lines.push(format!("More results: search again with page_token={next}"));
    }
    Ok(lines.join("\n"))
}

/// A message on two lines: who, when and what about, then the start of it.
fn summary_line(msg: &Value) -> String {
    format!(
        "{} · {} · {} · {}\n   {}",
        msg["id"].as_str().unwrap_or("?"),
        header(msg, "Date"),
        header(msg, "From"),
        header(msg, "Subject"),
        msg["snippet"]
            .as_str()
            .map(decode_entities)
            .unwrap_or_default()
    )
}

fn header<'a>(msg: &'a Value, name: &str) -> &'a str {
    msg["payload"]["headers"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|h| {
            h["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
        .and_then(|h| h["value"].as_str())
        .unwrap_or("")
}

/// A whole message in words: headers, attachments, and its text.
fn render_message(msg: &Value) -> String {
    let mut text = Vec::new();
    let mut html = Vec::new();
    let mut files = Vec::new();
    collect(&msg["payload"], &mut text, &mut html, &mut files);
    let body = if text.iter().any(|t| !t.trim().is_empty()) {
        text.join("\n")
    } else {
        crate::web::to_text(&html.join("\n"))
    };
    let mut out = format!(
        "Message {} (thread {})\n",
        msg["id"].as_str().unwrap_or("?"),
        msg["threadId"].as_str().unwrap_or("?")
    );
    for name in ["From", "To", "Cc", "Date", "Subject"] {
        let value = header(msg, name);
        if !value.is_empty() {
            out.push_str(&format!("{name}: {value}\n"));
        }
    }
    if !files.is_empty() {
        out.push_str(&format!("Attachments: {}\n", files.join(", ")));
    }
    out.push('\n');
    out.push_str(body.trim());
    out
}

/// Walks a message's parts, gathering plain text, HTML and attachment names.
fn collect(part: &Value, text: &mut Vec<String>, html: &mut Vec<String>, files: &mut Vec<String>) {
    let mime = part["mimeType"].as_str().unwrap_or("");
    let name = part["filename"].as_str().unwrap_or("");
    if !name.is_empty() {
        let size = part["body"]["size"].as_u64().unwrap_or(0);
        files.push(format!("{name} ({mime}, {size} bytes)"));
    } else if let Some(data) = part["body"]["data"].as_str() {
        let decoded = decode(data);
        match mime {
            "text/plain" => text.push(decoded),
            "text/html" => html.push(decoded),
            _ => {}
        }
    }
    for child in part["parts"].as_array().into_iter().flatten() {
        collect(child, text, html, files);
    }
}

/// Gmail's URL-safe base64, with or without padding.
fn decode(data: &str) -> String {
    URL_SAFE_NO_PAD
        .decode(data.trim_end_matches('='))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default()
}

/// The few entities Gmail leaves in a snippet.
fn decode_entities(s: &str) -> String {
    s.replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn render_labels(labels: &Value) -> String {
    let mut lines: Vec<String> = labels["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| {
            format!(
                "{} ({}): {}",
                l["name"].as_str().unwrap_or("?"),
                l["type"].as_str().unwrap_or("user"),
                l["id"].as_str().unwrap_or("?")
            )
        })
        .collect();
    lines.sort();
    if lines.is_empty() {
        "No labels.".to_owned()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn b64(s: &str) -> String {
        URL_SAFE_NO_PAD.encode(s)
    }

    #[test]
    fn a_message_shows_its_headers_text_and_attachments_but_not_the_html_twin() {
        let msg = json!({
            "id": "m1", "threadId": "t1",
            "payload": {
                "mimeType": "multipart/mixed",
                "headers": [
                    {"name": "from", "value": "Ann <ann@x.org>"},
                    {"name": "Subject", "value": "Lunch"}
                ],
                "parts": [
                    {"mimeType": "multipart/alternative", "parts": [
                        {"mimeType": "text/plain", "body": {"data": b64("Noon at the usual place.")}},
                        {"mimeType": "text/html", "body": {"data": b64("<p>Noon <b>html</b></p>")}}
                    ]},
                    {"mimeType": "application/pdf", "filename": "menu.pdf", "body": {"size": 2048, "attachmentId": "a"}}
                ]
            }
        });
        let out = render_message(&msg);
        assert!(out.starts_with("Message m1 (thread t1)\nFrom: Ann <ann@x.org>\nSubject: Lunch\n"));
        assert!(out.contains("Attachments: menu.pdf (application/pdf, 2048 bytes)"));
        assert!(out.ends_with("Noon at the usual place."));
        assert!(!out.contains("html"));
    }

    #[test]
    fn a_message_with_only_html_is_read_as_text() {
        let msg = json!({
            "id": "m2",
            "payload": {"mimeType": "text/html", "body": {"data": b64("<p>Hello <b>there</b></p>")}}
        });
        assert!(render_message(&msg).contains("Hello there"));
    }

    #[test]
    fn padded_and_unpadded_base64_both_decode() {
        assert_eq!(decode(&b64("ab")), "ab");
        assert_eq!(decode(&format!("{}==", b64("a"))), "a");
        assert_eq!(decode("!!"), "");
    }

    #[test]
    fn a_search_line_says_who_when_what_and_the_start() {
        let msg = json!({
            "id": "m3", "snippet": "Tom &amp; Jerry&#39;s",
            "payload": {"headers": [
                {"name": "Date", "value": "Mon, 3 Nov 2025"},
                {"name": "From", "value": "a@b.c"},
                {"name": "Subject", "value": "Hi"}
            ]}
        });
        assert_eq!(
            summary_line(&msg),
            "m3 · Mon, 3 Nov 2025 · a@b.c · Hi\n   Tom & Jerry's"
        );
    }

    #[test]
    fn an_id_that_could_change_the_address_is_refused() {
        assert!(id_arg(&json!({"id": "18c0a-F_1"})).is_ok());
        for bad in [
            json!({"id": "../labels"}),
            json!({"id": "a?b"}),
            json!({"id": ""}),
        ] {
            assert!(id_arg(&bad).is_err(), "{bad}");
        }
        assert_eq!(id_arg(&json!({})), Err(Failure::other("`id` is required")));
    }

    #[test]
    fn addresses_are_found_in_headers_with_names_and_quoted_commas() {
        assert_eq!(
            addresses_in(r#"Ann <a@x.org>, "Lee, Bo" <b@y.org>, c@z.org, nobody"#),
            ["a@x.org", "b@y.org", "c@z.org"]
        );
        assert!(addresses_in("").is_empty());
    }

    fn original() -> Value {
        json!({
            "threadId": "t9",
            "payload": {"headers": [
                {"name": "From", "value": "Ann <ann@x.org>"},
                {"name": "To", "value": "me@x.org, Bob <bob@y.org>"},
                {"name": "Cc", "value": "ANN@x.org, cy@z.org"},
                {"name": "Subject", "value": "Lunch"},
                {"name": "Message-ID", "value": "<m1@x>"},
                {"name": "References", "value": "<m0@x>"}
            ]}
        })
    }

    #[test]
    fn a_reply_answers_the_sender_in_the_same_thread() {
        let (mail, thread) = reply_to(&original(), "Sure", false, None).unwrap();
        assert_eq!(mail.to, ["ann@x.org"]);
        assert!(mail.cc.is_empty());
        assert_eq!(mail.subject, "Re: Lunch");
        assert_eq!(mail.in_reply_to.as_deref(), Some("<m1@x>"));
        assert_eq!(mail.references.as_deref(), Some("<m0@x> <m1@x>"));
        assert_eq!(thread, "t9");
    }

    #[test]
    fn reply_all_keeps_the_others_but_not_the_person_nor_the_sender_twice() {
        let (mail, _) = reply_to(&original(), "Sure", true, Some("ME@x.org")).unwrap();
        assert_eq!(mail.to, ["ann@x.org"]);
        assert_eq!(mail.cc, ["bob@y.org", "cy@z.org"]);
    }

    #[test]
    fn a_reply_to_header_wins_and_re_is_not_stacked() {
        let mut o = original();
        o["payload"]["headers"][3]["value"] = json!("RE: Lunch");
        o["payload"]["headers"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "Reply-To", "value": "list@x.org"}));
        let (mail, _) = reply_to(&o, "x", false, None).unwrap();
        assert_eq!(
            (mail.to, mail.subject.as_str()),
            (vec!["list@x.org".to_owned()], "RE: Lunch")
        );
        assert!(reply_to(&json!({"payload": {"headers": []}}), "x", false, None).is_err());
    }

    #[test]
    fn a_message_needs_recipients_a_subject_and_a_body_and_ids_stay_plain() {
        let m = compose(&json!({"to": "a@x.org, b@y.org", "subject": "S", "body": "B"})).unwrap();
        assert_eq!(m.to, ["a@x.org", "b@y.org"]);
        assert!(compose(&json!({"to": ["a@x.org"], "subject": "S"})).is_err());
        let body = message_body(&m, &json!({"thread_id": "t1"})).unwrap();
        assert_eq!(body["threadId"], "t1");
        assert!(message_body(&m, &json!({"thread_id": "../x"})).is_err());
        assert!(words(&json!({"add": ["INBOX", "Label_1"]}), "add").is_ok());
        assert!(words(&json!({"add": ["a b"]}), "add").is_err());
        assert!(word(&json!({}), "draft_id").is_err());
    }

    #[test]
    fn the_tools_that_change_things_are_marked_and_the_reads_are_not() {
        let tools = Gmail.tools();
        let changing: Vec<&str> = tools
            .iter()
            .filter(|t| t.access == Access::Changes)
            .map(|t| t.name)
            .collect();
        assert_eq!(
            changing,
            [
                "create_draft",
                "send_message",
                "send_draft",
                "reply",
                "modify_labels",
                "archive",
                "mark_read",
                "trash"
            ]
        );
        assert_eq!(
            tools.iter().filter(|t| t.access == Access::Reads).count(),
            4
        );
    }

    #[test]
    fn labels_are_listed_by_name() {
        let labels = json!({"labels": [
            {"id": "L2", "name": "Work", "type": "user"},
            {"id": "INBOX", "name": "INBOX", "type": "system"}
        ]});
        assert_eq!(
            render_labels(&labels),
            "INBOX (system): INBOX\nWork (user): L2"
        );
        assert_eq!(render_labels(&json!({})), "No labels.");
    }

    #[test]
    fn every_tool_has_a_name_a_description_and_an_object_schema() {
        for t in Gmail.tools() {
            assert!(!t.name.is_empty() && !t.description.is_empty());
            assert_eq!(t.schema["type"], "object", "{}", t.name);
        }
    }
}

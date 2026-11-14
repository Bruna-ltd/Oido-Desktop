//! Gmail: search and read mail, and see the labels. Read only; writing comes later.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};

use super::google::{Api, Failure};
use super::{Access, Extension, ToolDef};
use crate::mcp::catalog::Logo;

const READ: &str = "https://www.googleapis.com/auth/gmail.readonly";
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
        "Search and read the person's mail."
    }

    fn scopes(&self) -> &'static [&'static str] {
        &[READ]
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
            other => Err(Failure::other(format!("Gmail has no tool called {other}"))),
        }
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

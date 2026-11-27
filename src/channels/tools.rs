//! Channel tools: while a channel is connected, its tools are a set agents can be given, like an
//! extension's (`mcp__telegram__send_message`). An agent with them sends a message or a file to
//! someone on that channel from any chat, and lists who it can reach there.
//!
//! A chat carried on from a channel has that channel's `send_message` and `send_file` even when
//! its agent was not given them, but only for the person it talks to there.
//!
//! Who `to` names: a paired person (by name, handle or id), or anyone the channel can reach by
//! their id there. Left out, it is the person carrying the chat on from this channel, else the one
//! paired person marked trusted on the Channels screen: you.

use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use super::send::{self, Note, Upload};
use super::{Channel, Kind, Place};
use crate::app::App;
use crate::command::Finish;
use crate::completion::ToolSpec;
use crate::mcp::{Auth, Server, Status, Tool, Transport};

pub const SEND_MESSAGE: &str = "send_message";
pub const SEND_FILE: &str = "send_file";
pub const LIST_CONTACTS: &str = "list_contacts";

/// The server that stands for `kind`'s tools.
fn server(kind: Kind) -> Server {
    let mut tools = vec![Tool {
        name: SEND_MESSAGE.into(),
        description: format!(
            "Send a message on {} to someone, written in Markdown. Leave `to` out to send it to the person you talk to there, or else to the person who owns this Oido. Use it to tell them something when they are not at this computer: a result, a reminder, that a long task is done.",
            kind.title()
        ),
        schema: json!({
            "type": "object",
            "properties": {
                "to": {"type": "string", "description": to_hint(kind)},
                "text": {"type": "string", "description": "The message."}
            },
            "required": ["text"]
        }),
    }];
    if send::takes_files(kind) {
        tools.push(Tool {
            name: SEND_FILE.into(),
            description: format!(
                "Send a file from your working folder to someone privately on {}: a picture (PNG, JPEG, WebP) shows as a photo, anything else comes as a file to download. Up to {} MB. Leave `to` out to send it to the person you talk to there, or else to the person who owns this Oido.",
                kind.title(),
                send::max_file(kind) / (1024 * 1024)
            ),
            schema: json!({
                "type": "object",
                "properties": {
                    "to": {"type": "string", "description": to_hint(kind)},
                    "path": {"type": "string", "description": "The file, relative to the working folder."},
                    "caption": {"type": "string", "description": "A line to send with it."}
                },
                "required": ["path"]
            }),
        });
    }
    tools.push(Tool {
        name: LIST_CONTACTS.into(),
        description: format!(
            "List the people paired with Oido on {}, whom `to` can name, and which of them is the owner.",
            kind.title()
        ),
        schema: json!({"type": "object", "properties": {}}),
    });
    Server {
        id: format!("channel:{}", kind.as_str()),
        name: kind.title().to_owned(),
        transport: Transport::Channel(kind),
        auth: Auth::None,
        enabled: true,
        status: Status::Reachable,
        detail: String::new(),
        tools,
    }
}

/// What `to` takes on `kind`.
fn to_hint(kind: Kind) -> String {
    let raw = match kind {
        Kind::Telegram => "a Telegram chat id, or a public channel's @name",
        Kind::Slack => "a Slack user or channel id (U…, C…)",
        Kind::WhatsApp => "a phone number with its country code",
    };
    format!(
        "Who gets it: a paired person's name or handle (see list_contacts), or {raw}. Leave it out for the person you talk to there, or else the owner."
    )
}

/// The name a model calls `tool` of `kind` by.
fn qualified(kind: Kind, tool: &str) -> String {
    server(kind).qualified(tool)
}

/// The channel tool `name` is one of `kind`'s, with the tool's own name.
fn parse(name: &str) -> Option<(Kind, &str)> {
    Kind::ALL.into_iter().find_map(|kind| {
        let rest = name.strip_prefix(&qualified(kind, ""))?;
        [SEND_MESSAGE, SEND_FILE, LIST_CONTACTS]
            .into_iter()
            .find(|t| *t == rest)
            .map(|t| (kind, t))
    })
}

/// Whether `name` sends a file from the working folder (so the folder rules apply to its path).
pub fn sends_file(name: &str) -> bool {
    parse(name).is_some_and(|(_, t)| t == SEND_FILE)
}

/// Whether what `name` gives back is the app's own words, not outside text: what a send says
/// about how it went.
pub fn says_own(name: &str) -> bool {
    parse(name).is_some_and(|(_, t)| t != LIST_CONTACTS)
}

/// Someone a message goes to, found.
#[derive(Debug, PartialEq)]
struct Recipient {
    place: Place,
    label: String,
}

/// Who `to` names on `c` (`kind`), or why no one; `bound` is the person carrying the chat on from
/// this channel, if anyone.
fn recipient(
    kind: Kind,
    c: &Channel,
    to: Option<&str>,
    bound: Option<&str>,
) -> Result<Recipient, String> {
    let person = |id: &str| {
        let label = c.user(id).map_or_else(|| id.to_owned(), |p| p.name.clone());
        Recipient {
            place: Place::private(id),
            label,
        }
    };
    let Some(to) = to.map(str::trim).filter(|t| !t.is_empty()) else {
        if let Some(id) = bound {
            return Ok(person(id));
        }
        let owners: Vec<_> = c.users.iter().filter(|u| u.trusted).collect();
        return match owners.as_slice() {
            [one] => Ok(person(&one.id)),
            [] => Err(format!(
                "no one paired on {} is marked as the owner (trusted on the Channels screen), so say who with `to`.",
                kind.title()
            )),
            many => Err(format!(
                "several people on {} are trusted ({}), so say who with `to`.",
                kind.title(),
                many.iter()
                    .map(|u| u.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        };
    };
    let bare = to.strip_prefix('@').unwrap_or(to);
    if let Some(p) = c.users.iter().find(|u| {
        u.id == to
            || u.name.eq_ignore_ascii_case(to)
            || u.handle
                .as_deref()
                .is_some_and(|h| h.trim_start_matches('@').eq_ignore_ascii_case(bare))
    }) {
        return Ok(person(&p.id));
    }
    let chat = raw_id(kind, to).ok_or_else(|| {
        format!(
            "{to} is no one paired on {}, nor {}. list_contacts says who is paired.",
            kind.title(),
            match kind {
                Kind::Telegram => "a chat id or a public @name",
                Kind::Slack => "a Slack id",
                Kind::WhatsApp => "a phone number",
            }
        )
    })?;
    Ok(Recipient {
        place: Place::private(&chat),
        label: to.to_owned(),
    })
}

/// `to` as `kind` names a chat by itself, if it is written that way.
fn raw_id(kind: Kind, to: &str) -> Option<String> {
    match kind {
        Kind::Telegram => {
            let number = to.strip_prefix('-').unwrap_or(to);
            let is_number = !number.is_empty() && number.chars().all(|c| c.is_ascii_digit());
            let name = to.strip_prefix('@').filter(|n| {
                n.len() >= 4 && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
            (is_number || name.is_some()).then(|| to.to_owned())
        }
        Kind::Slack => {
            let first = to.chars().next()?;
            (to.len() >= 9
                && matches!(first, 'U' | 'W' | 'C' | 'G' | 'D')
                && to
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
            .then(|| to.to_owned())
        }
        Kind::WhatsApp => {
            if let Some((user, server)) = to.split_once('@') {
                return (!user.is_empty() && !server.is_empty()).then(|| to.to_owned());
            }
            let digits: String = to
                .chars()
                .filter(|c| !matches!(c, '+' | ' ' | '-' | '(' | ')' | '.'))
                .collect();
            ((7..=15).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit()))
                .then(|| format!("{digits}@s.whatsapp.net"))
        }
    }
}

/// The people paired on `c`, in words for the model.
fn contacts(kind: Kind, c: &Channel) -> String {
    if c.users.is_empty() {
        return format!(
            "No one is paired on {} yet. {}",
            kind.title(),
            to_hint(kind)
        );
    }
    let lines: Vec<String> = c
        .users
        .iter()
        .map(|u| {
            let handle = u
                .handle
                .as_deref()
                .map(|h| format!(" (@{})", h.trim_start_matches('@')))
                .unwrap_or_default();
            let owner = if u.trusted { ", the owner" } else { "" };
            format!("- {}{handle}: id {}{owner}", u.name, u.id)
        })
        .collect();
    format!(
        "Paired on {}:\n{}\n`to` also takes {}.",
        kind.title(),
        lines.join("\n"),
        match kind {
            Kind::Telegram => "a chat id, or a public channel's @name, for chats the bot is in",
            Kind::Slack => "a Slack user or channel id the bot can post to",
            Kind::WhatsApp => "any phone number with its country code",
        }
    )
}

impl App {
    /// Makes each connected channel a server in the register, offering its tools. Never saved
    /// with the person's own servers.
    pub(crate) fn rebuild_channel_servers(&mut self) {
        self.mcp
            .servers
            .retain(|s| !matches!(s.transport, Transport::Channel(_)));
        for kind in Kind::ALL {
            if self.channels.get(kind).connected() {
                self.mcp.servers.push(server(kind));
            }
        }
    }

    /// The channel tools chat `sid` has without its agent being given them: `send_message` and
    /// `send_file` of the channel it is carried on from, for the person it talks to there.
    pub(crate) fn bound_channel_tools(&self, sid: &str) -> Vec<ToolSpec> {
        let Some(kind) = self.channels.kind_of(sid) else {
            return Vec::new();
        };
        let files = self.channels.files_go(sid).is_some();
        let wanted = [qualified(kind, SEND_MESSAGE), qualified(kind, SEND_FILE)];
        crate::mcp::specs(&[server(kind)])
            .into_iter()
            .filter(|t| t.name == wanted[0] || (files && t.name == wanted[1]))
            .collect()
    }

    /// The agent called `kind`'s `tool`. A send runs as a job; the list is answered at once. Says
    /// why when it cannot start.
    pub(crate) fn start_channel_call(
        &mut self,
        sid: &str,
        call_id: &str,
        kind: Kind,
        tool: &str,
        args: &Value,
    ) -> Result<(), String> {
        let c = self.channels.get(kind);
        if tool == LIST_CONTACTS {
            let text = contacts(kind, c);
            self.resolve(sid, call_id, text, true);
            self.start_next_command(sid);
            return Ok(());
        }
        let name = qualified(kind, tool);
        let chosen = self
            .roster
            .owner_of(sid)
            .is_some_and(|a| a.tools.contains(&name));
        let bound = (self.channels.kind_of(sid) == Some(kind))
            .then(|| c.user_of(sid))
            .flatten();
        let to = args.get("to").and_then(Value::as_str);
        let who = recipient(kind, c, to, bound.as_deref())?;
        // Offered only because the chat is carried on from here: for its own person alone.
        if !chosen && bound.as_deref() != Some(who.place.chat.as_str()) {
            return Err(format!(
                "here {name} reaches only the person you talk to. The person can give you {} tools on the agent's page to reach others.",
                kind.title()
            ));
        }
        let outbox = c.link.as_ref().map(super::Link::outbox);
        let not_listening = || format!("the {} bot is not listening now.", kind.title());
        let job: Box<dyn FnOnce() -> String + Send> = match tool {
            SEND_MESSAGE => {
                let text = args
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .ok_or("`text` is empty.")?
                    .to_owned();
                let note = Note {
                    kind,
                    to: who.place,
                    label: who.label,
                    text,
                    outbox: outbox.ok_or_else(not_listening)?,
                };
                Box::new(move || note.send())
            }
            SEND_FILE => {
                let upload = self.upload(sid, kind, who, args, outbox)?;
                Box::new(move || upload.send())
            }
            _ => return Err(format!("{} has no tool called {tool}.", kind.title())),
        };
        let cancel = self.track(sid, call_id);
        let (sid, call) = (sid.to_owned(), call_id.to_owned());
        self.spawn(
            move || (job(), cancel.load(Ordering::Relaxed)),
            move |app, (said, stopped)| {
                let finish = if stopped {
                    Finish::Stopped("Stopped; it may have been sent already.".into())
                } else {
                    Finish::Done(said)
                };
                app.command_done(&sid, &call, finish);
            },
        );
        Ok(())
    }

    /// The file a `send_file` call names, checked: in the working folder, and small enough.
    fn upload(
        &self,
        sid: &str,
        kind: Kind,
        who: Recipient,
        args: &Value,
        outbox: Option<std::sync::mpsc::Sender<super::Out>>,
    ) -> Result<Upload, String> {
        let c = self.channels.get(kind);
        let creds = c
            .creds
            .clone()
            .ok_or_else(|| format!("the {} bot is not listening now.", kind.title()))?;
        let given = args.get("path").and_then(Value::as_str).unwrap_or_default();
        let path = crate::files::resolve(&self.workdir_of(sid), given)?;
        let meta = std::fs::metadata(&path).map_err(|_| format!("{given} does not exist."))?;
        if !meta.is_file() {
            return Err(format!("{given} is not a file."));
        }
        let most = send::max_file(kind);
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
            to: who.place,
            label: who.label,
            path,
            name,
            caption: args
                .get("caption")
                .and_then(Value::as_str)
                .map(str::to_owned),
            outbox,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::channels::Paired;

    fn person(id: &str, name: &str, handle: Option<&str>, trusted: bool) -> Paired {
        Paired {
            id: id.into(),
            name: name.into(),
            handle: handle.map(str::to_owned),
            agent: None,
            trusted,
        }
    }

    fn channel(users: Vec<Paired>) -> Channel {
        Channel {
            users,
            ..Channel::default()
        }
    }

    fn chat(r: Result<Recipient, String>) -> String {
        r.map(|r| r.place.chat)
            .unwrap_or_else(|e| format!("Error: {e}"))
    }

    #[test]
    fn tool_names_are_told_apart_from_other_servers_and_each_other() {
        assert_eq!(
            qualified(Kind::WhatsApp, SEND_MESSAGE),
            "mcp__whatsapp__send_message"
        );
        assert!(sends_file("mcp__telegram__send_file"));
        assert!(!sends_file("mcp__telegram__send_message") && !sends_file("mcp__files__send_file"));
        assert!(says_own("mcp__slack__send_message"));
        assert!(!says_own("mcp__slack__list_contacts"), "names people chose");
        assert!(!says_own("mcp__mail__send_message"));
    }

    #[test]
    fn to_left_out_is_the_person_talked_to_else_the_one_trusted_person() {
        let c = channel(vec![
            person("1", "Ada", Some("ada"), true),
            person("2", "Bob", None, false),
        ]);
        let t = Kind::Telegram;
        assert_eq!(chat(recipient(t, &c, None, Some("2"))), "2");
        assert_eq!(chat(recipient(t, &c, Some(" "), None)), "1");
        let none = channel(vec![person("2", "Bob", None, false)]);
        assert!(chat(recipient(t, &none, None, None)).contains("say who with `to`"));
        let two = channel(vec![
            person("1", "Ada", None, true),
            person("3", "Cy", None, true),
        ]);
        assert!(chat(recipient(t, &two, None, None)).contains("Ada, Cy"));
    }

    #[test]
    fn to_names_a_paired_person_or_an_id_the_channel_takes() {
        let c = channel(vec![person("1", "Ada Lovelace", Some("ada"), false)]);
        let t = Kind::Telegram;
        for name in ["ada lovelace", "@ada", "ADA", "1"] {
            assert_eq!(chat(recipient(t, &c, Some(name), None)), "1", "{name}");
        }
        let r = recipient(t, &c, Some("Ada Lovelace"), None).unwrap();
        assert_eq!(r.label, "Ada Lovelace");
        assert_eq!(chat(recipient(t, &c, Some("-1001234"), None)), "-1001234");
        assert_eq!(
            chat(recipient(t, &c, Some("@news_room"), None)),
            "@news_room"
        );
        assert!(chat(recipient(t, &c, Some("Grace"), None)).starts_with("Error: Grace is no one"));

        let s = Kind::Slack;
        assert_eq!(chat(recipient(s, &c, Some("C0123ABCD"), None)), "C0123ABCD");
        assert!(chat(recipient(s, &c, Some("#general"), None)).starts_with("Error:"));

        let w = Kind::WhatsApp;
        assert_eq!(
            chat(recipient(w, &c, Some("+357 99 123-456"), None)),
            "35799123456@s.whatsapp.net"
        );
        assert_eq!(chat(recipient(w, &c, Some("123@lid"), None)), "123@lid");
        assert!(chat(recipient(w, &c, Some("12"), None)).starts_with("Error:"));
    }

    #[test]
    fn the_contact_list_marks_the_owner() {
        let c = channel(vec![
            person("1", "Ada", Some("ada"), true),
            person("2", "Bob", None, false),
        ]);
        let said = contacts(Kind::Slack, &c);
        assert!(said.contains("- Ada (@ada): id 1, the owner"), "{said}");
        assert!(said.contains("- Bob: id 2\n"), "{said}");
        assert!(contacts(Kind::Slack, &channel(Vec::new())).starts_with("No one is paired"));
    }
}

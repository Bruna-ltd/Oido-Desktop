//! The log: every message a connected channel hears or sees, from anyone, kept 30 days for agents
//! to read (`read_messages`, see [`super::tools`]). Words only: a file is noted by its name, never
//! kept. It stays in the database on this computer; the Channels screen clears it.

use super::{Heard, Kind};
use crate::app::App;

/// How long a message is kept, in seconds: 30 days.
pub const KEEP_SECS: i64 = 30 * 24 * 60 * 60;
/// How often the log is cut to what is kept while the app runs, in seconds.
const PURGE_EVERY: i64 = 60 * 60;

/// A message as the log keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Logged {
    /// The chat it was in, as the channel names it (where a reply to it goes).
    pub chat: String,
    /// The chat's title where it has one of its own: a group's, a Slack channel's.
    pub chat_name: Option<String>,
    pub sender: String,
    pub sender_name: String,
    /// When it was sent, in seconds since the epoch.
    pub at: i64,
    pub text: String,
    /// The name of a file sent with it.
    pub file: Option<String>,
    /// The message's own id, where the channel has one.
    pub msg_id: Option<String>,
}

impl Logged {
    /// What the log keeps of `heard`: a message, not a command or a press.
    fn of(heard: &Heard) -> Option<Logged> {
        match heard {
            Heard::Message {
                from,
                place,
                id,
                at,
                text,
                picture,
            } => Some(Logged {
                chat: place.chat.clone(),
                chat_name: None,
                sender: from.id.clone(),
                sender_name: from.name.clone(),
                at: *at,
                text: text.clone(),
                file: picture.as_ref().map(|p| p.name.clone()),
                msg_id: id.clone(),
            }),
            Heard::Overheard(m) => Some(m.clone()),
            Heard::Command { .. } | Heard::Press { .. } => None,
        }
    }

    /// Where it was, in words: the chat's title, else whom it was with.
    pub fn place(&self) -> &str {
        self.chat_name.as_deref().unwrap_or(&self.sender_name)
    }
}

impl App {
    /// Keeps what `kind` heard at `now`, if it is a message.
    pub(super) fn log_heard(&mut self, kind: Kind, heard: &Heard, now: i64) {
        let Some(m) = Logged::of(heard) else { return };
        if m.text.trim().is_empty() && m.file.is_none() {
            return;
        }
        let Some(db) = &self.store else { return };
        if let Err(e) = db.log_channel_message(kind, &m) {
            eprintln!("oido: could not keep a {} message: {e}", kind.title());
        }
        if now - self.channels.purged >= PURGE_EVERY {
            self.purge_channel_log(now);
        } else {
            self.count_channel_log(kind);
        }
    }

    /// Forgets what is older than [`KEEP_SECS`] at `now`.
    pub(super) fn purge_channel_log(&mut self, now: i64) {
        self.channels.purged = now;
        if let Some(db) = &self.store
            && let Err(e) = db.purge_channel_log(now - KEEP_SECS)
        {
            eprintln!("oido: could not cut the channels' log: {e}");
        }
        for kind in Kind::ALL {
            self.count_channel_log(kind);
        }
    }

    fn count_channel_log(&mut self, kind: Kind) {
        let n = self
            .store
            .as_ref()
            .and_then(|db| db.channel_log_len(kind).ok())
            .unwrap_or(0);
        self.channels.get_mut(kind).logged = n;
    }

    /// The Clear button: forgets everything `kind` heard.
    pub fn clear_channel_log(&mut self, kind: Kind) {
        self.save("the log", |db| db.clear_channel_log(kind));
        self.channels.get_mut(kind).confirm_clear = false;
        self.count_channel_log(kind);
    }

    /// What `kind` heard since `since`, newest first, at most `limit`.
    pub(super) fn channel_log(&self, kind: Kind, since: i64, limit: usize) -> Vec<Logged> {
        self.store
            .as_ref()
            .and_then(|db| db.channel_log(kind, since, limit).ok())
            .unwrap_or_default()
    }
}

/// The most messages one read gives back.
pub const MAX_READ: usize = 200;
/// How many a read gives back unless told.
const READ: usize = 30;
/// The longest a message is shown, in characters; the rest is cut.
const SHOWN: usize = 1000;

/// `s` lowercased, without what a name is often written with: `#general`, `@ada`, `+357…`.
fn bare(s: &str) -> String {
    s.trim().trim_start_matches(['#', '@', '+']).to_lowercase()
}

/// Whether `m` was in the chat `want` names: by its id, its title, or for a private chat whom it
/// was with.
pub(super) fn in_chat(m: &Logged, want: &str) -> bool {
    let want = bare(want);
    bare(&m.chat) == want
        || m.chat_name.as_deref().is_some_and(|n| bare(n) == want)
        || (m.chat_name.is_none() && (bare(&m.sender_name) == want || bare(&m.sender) == want))
}

/// Whether `want` names whoever wrote `m`, by id or a part of their name.
fn by(m: &Logged, want: &str) -> bool {
    let want = bare(want);
    bare(&m.sender) == want || m.sender_name.to_lowercase().contains(&want)
}

/// When a read starts, from what the model wrote: `30m`, `2h`, `3d`, a day (`2026-11-27`), or a
/// local time (`2026-11-27 09:30`). Nothing given is as far back as the log keeps.
pub(super) fn since(given: Option<&str>, now: i64) -> Result<i64, String> {
    let Some(given) = given.map(str::trim).filter(|g| !g.is_empty()) else {
        return Ok(now - KEEP_SECS);
    };
    let unit = match given.chars().last() {
        Some('m') => Some(60),
        Some('h') => Some(60 * 60),
        Some('d') => Some(24 * 60 * 60),
        _ => None,
    };
    if let Some(unit) = unit
        && let Ok(n) = given[..given.len() - 1].trim().parse::<i64>()
    {
        return Ok(now - n.max(0) * unit);
    }
    let at = if given.len() == 10 {
        format!("{given} 00:00")
    } else {
        given.to_owned()
    };
    crate::schedule::tool::local_time(&at, &chrono::Local).map_err(|_| {
        format!(
            "`since` {given:?} is not like \"2h\", \"3d\", \"2026-11-27\" or \"2026-11-27 09:30\"."
        )
    })
}

/// A moment, in the local zone, for the model.
pub(super) fn stamp(at: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(at, 0).single().map_or_else(
        || at.to_string(),
        |t| t.format("%Y-%m-%d %H:%M").to_string(),
    )
}

/// What a read asks for.
pub(super) struct Read<'a> {
    pub chat: Option<&'a str>,
    pub from: Option<&'a str>,
    pub since: i64,
    pub limit: usize,
}

impl Read<'_> {
    /// `limit` as asked, within what one read gives.
    pub(super) fn limit(asked: Option<u64>) -> usize {
        asked
            .map_or(READ, |n| usize::try_from(n).unwrap_or(MAX_READ))
            .clamp(1, MAX_READ)
    }

    /// The messages of `all` (newest first) it asks for, oldest first.
    pub(super) fn pick(&self, all: Vec<Logged>) -> Vec<Logged> {
        let mut got: Vec<Logged> = all
            .into_iter()
            .filter(|m| m.at >= self.since)
            .filter(|m| self.chat.is_none_or(|c| in_chat(m, c)))
            .filter(|m| self.from.is_none_or(|f| by(m, f)))
            .take(self.limit)
            .collect();
        got.reverse();
        got
    }
}

/// `messages` (oldest first) in words for the model, chat by chat; `where_` says where they are
/// from, `empty` what to say when there are none.
pub(super) fn render(messages: &[Logged], where_: &str, empty: &str) -> String {
    if messages.is_empty() {
        return empty.to_owned();
    }
    let mut chats: Vec<(&str, Vec<&Logged>)> = Vec::new();
    for m in messages {
        match chats.iter_mut().find(|(c, _)| *c == m.chat) {
            Some((_, list)) => list.push(m),
            None => chats.push((&m.chat, vec![m])),
        }
    }
    let mut out = format!(
        "{} message{} {where_}, oldest first. To answer one, send_message with `to` set to its chat id.",
        messages.len(),
        if messages.len() == 1 { "" } else { "s" }
    );
    for (chat, list) in chats {
        out.push_str(&format!("\n\n## {} (chat id {chat})", list[0].place()));
        for m in list {
            let mut text: String = m.text.trim().chars().take(SHOWN).collect();
            if m.text.trim().chars().count() > SHOWN {
                text.push('…');
            }
            let file = m
                .file
                .as_deref()
                .map(|f| format!(" [sent {f}]"))
                .unwrap_or_default();
            out.push_str(&format!(
                "\n- {} {}: {text}{file}",
                stamp(m.at),
                m.sender_name
            ));
        }
    }
    out
}

/// The chats in `all` (newest first), most recent first, at most `most`: each once, with how
/// many messages and when the last was.
pub(super) fn chats(all: &[Logged], most: usize) -> Vec<(&Logged, usize)> {
    let mut seen: Vec<(&Logged, usize)> = Vec::new();
    for m in all {
        match seen.iter_mut().find(|(c, _)| c.chat == m.chat) {
            Some((_, n)) => *n += 1,
            None => seen.push((m, 1)),
        }
    }
    seen.truncate(most);
    seen
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn m(chat: &str, title: Option<&str>, who: &str, at: i64, text: &str) -> Logged {
        Logged {
            chat: chat.into(),
            chat_name: title.map(str::to_owned),
            sender: format!("id-{who}"),
            sender_name: who.into(),
            at,
            text: text.into(),
            file: None,
            msg_id: None,
        }
    }

    #[test]
    fn a_chat_is_named_by_its_id_its_title_or_whom_it_was_with() {
        let group = m("g1@g.us", Some("Family"), "Ada", 1, "hi");
        let dm = m("42", None, "Bob Stone", 1, "yo");
        assert!(in_chat(&group, "family") && in_chat(&group, "g1@g.us"));
        assert!(!in_chat(&group, "Ada"), "a group is not its writer's");
        assert!(in_chat(&dm, "@bob stone") && in_chat(&dm, "42"));
        let slack = m("C1", Some("#general"), "Ada", 1, "x");
        assert!(in_chat(&slack, "general") && in_chat(&slack, "#general"));
    }

    #[test]
    fn a_read_takes_the_newest_it_asks_for_and_gives_them_oldest_first_by_chat() {
        let all = vec![
            m("g", Some("Family"), "Ada", 40, "four"),
            m("42", None, "Bob", 30, "three"),
            m("g", Some("Family"), "Cy", 20, "two"),
            m("g", Some("Family"), "Ada", 10, "one"),
        ];
        let read = |chat, from, since, limit| {
            Read {
                chat,
                from,
                since,
                limit,
            }
            .pick(all.clone())
            .iter()
            .map(|m| m.text.clone())
            .collect::<Vec<_>>()
        };
        assert_eq!(read(None, None, 0, 3), ["two", "three", "four"]);
        assert_eq!(read(Some("family"), Some("ada"), 0, 9), ["one", "four"]);
        assert_eq!(read(None, None, 25, 9), ["three", "four"]);
        let said = render(
            &Read {
                chat: None,
                from: None,
                since: 0,
                limit: 9,
            }
            .pick(all.clone()),
            "on WhatsApp",
            "none",
        );
        assert!(
            said.starts_with("4 messages on WhatsApp, oldest first."),
            "{said}"
        );
        let family = said.find("## Family (chat id g)").unwrap();
        let bob = said.find("## Bob (chat id 42)").unwrap();
        assert!(family < bob, "{said}");
        assert!(said.contains(" Cy: two\n"), "{said}");
        assert_eq!(render(&[], "x", "none"), "none");
        assert_eq!(
            chats(&all, 9)
                .iter()
                .map(|(c, n)| (c.chat.as_str(), *n))
                .collect::<Vec<_>>(),
            [("g", 3), ("42", 1)]
        );
    }

    #[test]
    fn since_is_relative_a_day_or_a_time() {
        let now = 1_000_000;
        assert_eq!(since(None, now).unwrap(), now - KEEP_SECS);
        assert_eq!(since(Some("2h"), now).unwrap(), now - 7200);
        assert_eq!(since(Some("3d"), now).unwrap(), now - 3 * 86400);
        assert_eq!(since(Some("30m"), now).unwrap(), now - 1800);
        let day = since(Some("2026-11-27"), now).unwrap();
        assert_eq!(since(Some("2026-11-27 00:00"), now).unwrap(), day);
        assert!(since(Some("last week"), now).is_err());
    }
}

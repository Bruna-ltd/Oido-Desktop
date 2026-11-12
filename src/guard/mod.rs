//! Guardrails: checks on this computer, on both sides of the model.
//!
//! Going out, every request passes through [`Guard::request`], the only way to make the
//! [`Guarded`] request a model call takes: secrets in it are replaced with numbered placeholders
//! (see [`secrets::Hidden`]). Coming back, what the model wrote is revealed ([`Hidden::reveal`]:
//! placeholders turned back into the values) before it is saved or acted on, and each tool call it asked for is
//! reviewed by [`Guard::review`] before it runs: a risky one is blocked, or held for the person's
//! go-ahead (see [`Trip`]). Chats keep the real values; only models see placeholders.
//!
//! The rules come in packs, each switched on or off on the Guardrails screen (see [`Pack`]).

pub mod actions;
pub mod ask;
mod gate;
pub mod injection;
mod intake;
pub mod replies;
pub mod secrets;
pub mod taint;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use self::actions::Place;
pub use self::intake::topics;
pub use self::secrets::Hidden;
use crate::completion::Request;
use crate::model::Message;
use crate::store::Store;

/// Where the packs' switches are saved.
const PACKS: &str = "guard.packs";
/// Where how long a go-ahead is waited for is saved, in minutes.
const WAIT: &str = "guard.wait";
/// Where the blocked words are saved.
const WORDS: &str = "guard.words";
/// Where the limits on channel messages are saved.
const LIMITS: &str = "guard.channel_limits";

/// The longest channel message choices on the Guardrails screen, in characters (0: no limit).
pub const LENGTHS: [u32; 4] = [0, 500, 2000, 8000];
/// The messages-an-hour choices for one channel person on the Guardrails screen (0: no limit).
pub const RATES: [u32; 4] = [0, 10, 30, 100];

/// How long a call held for a go-ahead waits, in minutes, until it is changed on the Guardrails
/// screen; after it, the call does not run.
const DEFAULT_WAIT: u32 = 10;

/// The waits offered on the Guardrails screen, in minutes.
pub const WAITS: [u32; 4] = [5, 10, 30, 60];

/// What the model is told when values are hidden from it.
const PLACEHOLDER_NOTE: &str = "Some values, such as keys, passwords, personal details and certain words, are hidden from you as placeholders like <<SECRET_1>>, <<EMAIL_2>> or <<WORD_3>>. Where a value is needed, in a tool's arguments too, write its placeholder exactly as it is: it is replaced with the real value on the person's computer. Never guess or ask for a hidden value.";

/// A group of rules switched on or off together.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pack {
    /// Keys, tokens, private keys and passwords never reach a model.
    Secrets,
    /// Commands and file changes that can do lasting harm wait for a go-ahead, or are refused.
    Actions,
    /// While outside text is in what a model is sent, every call that acts waits for a go-ahead.
    Taint,
    /// Invisible characters are taken out of what a model is sent, and text that tries to give
    /// orders counts as outside text whatever its source.
    Injection,
    /// Email addresses, phone numbers, card numbers and IBANs in outside text never reach a model.
    Personal,
    /// Replies to channel people are checked: no echo of the brief, no paths or internal addresses.
    Replies,
}

impl Pack {
    pub const ALL: [Pack; 6] = [
        Pack::Secrets,
        Pack::Actions,
        Pack::Taint,
        Pack::Injection,
        Pack::Personal,
        Pack::Replies,
    ];

    /// Whether the pack is on until switched: the ones that could get in the way start off.
    fn starts_on(self) -> bool {
        !matches!(self, Self::Personal | Self::Replies)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Secrets => "Hide secrets from models",
            Self::Actions => "Stop risky actions",
            Self::Taint => "Ask before acting on outside text",
            Self::Injection => "Catch hidden instructions",
            Self::Personal => "Hide personal details from models",
            Self::Replies => "Check replies to channels",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Self::Secrets => {
                "API keys, tokens, private keys and passwords are replaced with placeholders before anything goes to a model, and put back only on this computer, where tools use them."
            }
            Self::Actions => {
                "Commands and file changes that can do lasting harm wait for your go-ahead: running as administrator, deleting outside the working folder, running code straight from the internet, changing login or startup files. Erasing a disk is refused."
            }
            Self::Taint => {
                "While an agent has a web page, an MCP tool's result or a message from someone not trusted in front of it, it asks before it runs a command, changes a file, fetches a page, sends something or uses another tool that acts, so instructions hidden in that text cannot act on their own."
            }
            Self::Injection => {
                "Characters you cannot see are taken out of what goes to a model, and text that tries to give the agent orders (\"ignore your previous instructions\", a fake system message) counts as outside text even from a folder or person you trust. MCP tools' descriptions are checked too."
            }
            Self::Personal => {
                "Email addresses, international phone numbers, card numbers and IBANs in outside text (web pages, MCP results, messages from people not trusted) are replaced with placeholders before anything goes to a model, and put back on this computer."
            }
            Self::Replies => {
                "A reply to someone on a channel is held back if it repeats the agent's instructions, and paths on this computer and internal addresses (localhost, 192.168…) in it are left out."
            }
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Secrets => "secrets",
            Self::Actions => "actions",
            Self::Taint => "taint",
            Self::Injection => "injection",
            Self::Personal => "personal",
            Self::Replies => "replies",
        }
    }
}

/// What happens when a rule is broken.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Refused: it does not run.
    Block,
    /// Held until the person says yes or no.
    Ask,
}

/// A guardrail a tool call tripped, kept with the call in the chat.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Trip {
    /// The rule's id (`shell.admin`); [`title`] names it.
    pub rule: String,
    /// Why, in words, for the person and the model.
    pub why: String,
    pub state: TripState,
    /// When it began waiting for a go-ahead (Unix seconds), so it can stop waiting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked_at: Option<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TripState {
    /// Refused by the rule; it never ran.
    Blocked,
    /// Waiting for the person's go-ahead.
    Asking,
    Allowed,
    Denied,
    /// Nobody answered in time, so it did not run.
    TimedOut,
}

impl TripState {
    /// Whether the call was stopped: it never ran and never will.
    pub fn stopped(self) -> bool {
        matches!(self, Self::Blocked | Self::Denied | Self::TimedOut)
    }
}

impl Trip {
    /// What the model is told when the call does not run.
    pub fn told(&self) -> String {
        let rule = title(&self.rule);
        match self.state {
            TripState::Denied => {
                format!("Not run: the person said no. ({rule}: {})", self.why)
            }
            TripState::TimedOut => format!(
                "Not run: nobody gave the go-ahead in time, so it was not allowed. ({rule}: {})",
                self.why
            ),
            _ => format!(
                "Not run: a guardrail stopped it. {rule}: {} Tell the person, and do not try to get around it.",
                self.why
            ),
        }
    }
}

/// What the model is told when a call it made while working on its own (a team member, a workflow
/// step) breaks a rule: nobody is there to give a go-ahead, so it does not run.
pub fn refused_unattended(f: &actions::Finding) -> String {
    let rule = title(f.rule);
    match f.action {
        Action::Block => format!(
            "Not run: a guardrail stopped it. {rule}: {} Do not try to get around it.",
            f.why
        ),
        Action::Ask => format!(
            "Not run: a guardrail stopped it. {rule}: {} It needs the person's go-ahead, which cannot be given while working on your own. Say what you wanted to run in your answer instead.",
            f.why
        ),
    }
}

/// A call made by work nobody watches (a team member, a workflow step) broke rule `f`: refused
/// outright, or put to the person through `asker` when there is anyone to ask. Returns how it
/// ended (allowed, refused, turned down, unanswered) and, when it does not run, what the model is
/// told.
pub fn unattended(
    asker: &ask::Asker,
    name: &str,
    args: &str,
    f: &actions::Finding,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> (TripState, Option<String>) {
    if f.action == Action::Block || !asker.can_ask() {
        return (TripState::Blocked, Some(refused_unattended(f)));
    }
    let state = asker.ask(name, &crate::tools::summary(name, args), f, stop);
    let told = (state != TripState::Allowed).then(|| {
        Trip {
            rule: f.rule.to_owned(),
            why: f.why.clone(),
            state,
            asked_at: None,
        }
        .told()
    });
    (state, told)
}

/// A rule's name, for the person.
pub fn title(rule: &str) -> &'static str {
    match rule {
        "secrets.unknown" => "Unknown hidden value",
        "secrets.off_machine" => "Sends a secret away",
        "taint.outside" => "Acts after reading outside text",
        other => actions::title(other),
    }
}

/// A request that has been through the guard, and so may be sent to a model.
#[derive(Clone, PartialEq, Debug)]
pub struct Guarded {
    req: Request,
    /// How many invisible characters were taken out of it.
    stripped: usize,
}

impl Guarded {
    pub fn request(&self) -> &Request {
        &self.req
    }

    /// How many characters a person cannot see were taken out of it.
    pub fn stripped(&self) -> usize {
        self.stripped
    }
}

/// Limits on what one channel person may send (see `Guard::limits`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Limits {
    /// The longest message, in characters; 0 for no limit.
    #[serde(default)]
    pub length: u32,
    /// How many messages an hour; 0 for no limit.
    #[serde(default)]
    pub per_hour: u32,
}

/// The rules every check applies: which packs are on, how long a go-ahead is waited for, the
/// blocked words and the limits on channel messages.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Guard {
    /// Packs switched from how they start (see [`Pack::starts_on`]).
    packs: BTreeMap<Pack, bool>,
    /// How long a go-ahead is waited for, in minutes, when changed from [`DEFAULT_WAIT`].
    wait: Option<u32>,
    /// Words never sent to a model or shown to a channel person who did not write them.
    words: Vec<String>,
    limits: Limits,
}

/// What [`Guard`] looks for when it learns what to hide from a text.
struct Learner {
    secrets: bool,
    personal: bool,
    words: Option<regex::Regex>,
}

impl Learner {
    fn active(&self) -> bool {
        self.secrets || self.personal || self.words.is_some()
    }

    /// Notes what to hide in `text`: secrets and blocked words anywhere, personal details only in
    /// `outside` text.
    fn learn(&self, hidden: &mut Hidden, text: &str, outside: bool) {
        if self.secrets {
            hidden.learn(text);
        }
        if let Some(words) = &self.words {
            hidden.learn_words(text, words);
        }
        if self.personal && outside {
            hidden.learn_personal(text);
        }
    }

    /// [`Learner::learn`] over the strings of a tool call's arguments.
    fn learn_json(&self, hidden: &mut Hidden, args: &str, outside: bool) {
        for s in secrets::json_strings(args) {
            self.learn(hidden, &s, outside);
        }
    }
}

impl Guard {
    /// The packs as saved; every pack is on until turned off.
    pub fn load(store: Option<&Store>) -> Guard {
        let saved = store
            .map(|db| {
                db.setting::<BTreeMap<String, bool>>(PACKS)
                    .unwrap_or_else(|e| {
                        eprintln!("oido: could not read the guardrails: {e}");
                        None
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let read = |key: &str| {
            store.and_then(|db| {
                db.setting::<serde_json::Value>(key).unwrap_or_else(|e| {
                    eprintln!("oido: could not read the guardrails' {key}: {e}");
                    None
                })
            })
        };
        let wait = read(WAIT).and_then(|v| serde_json::from_value::<u32>(v).ok());
        Guard {
            packs: Pack::ALL
                .into_iter()
                .filter_map(|p| saved.get(p.key()).map(|on| (p, *on)))
                .collect(),
            wait: wait.filter(|w| *w > 0),
            words: read(WORDS)
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default(),
            limits: read(LIMITS)
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default(),
        }
    }

    /// The words never sent to a model or shown to a channel person who did not write them.
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// Sets the blocked words: each trimmed, empty ones and repeats left out.
    pub fn set_words(&mut self, words: Vec<String>) {
        let mut kept: Vec<String> = Vec::new();
        for w in words {
            let w = w.trim().to_owned();
            if !w.is_empty() && !kept.iter().any(|k| k.eq_ignore_ascii_case(&w)) {
                kept.push(w);
            }
        }
        self.words = kept;
    }

    /// The limits on what one channel person may send.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = limits;
    }

    fn learner(&self) -> Learner {
        Learner {
            secrets: self.on(Pack::Secrets),
            personal: self.on(Pack::Personal),
            words: secrets::words(&self.words),
        }
    }

    /// How long a call held for a go-ahead waits, in minutes.
    pub fn wait(&self) -> u32 {
        self.wait.unwrap_or(DEFAULT_WAIT)
    }

    pub fn set_wait(&mut self, minutes: u32) {
        self.wait = Some(minutes);
    }

    pub fn on(&self, pack: Pack) -> bool {
        self.packs
            .get(&pack)
            .copied()
            .unwrap_or_else(|| pack.starts_on())
    }

    pub fn set(&mut self, pack: Pack, on: bool) {
        self.packs.insert(pack, on);
    }

    /// The packs as saved.
    fn saved(&self) -> BTreeMap<&'static str, bool> {
        Pack::ALL.iter().map(|p| (p.key(), self.on(*p))).collect()
    }

    /// What to hide in a chat with brief `system`: what is in the brief first, then each value in
    /// the order the chat first has it, so the numbering of what was sent before does not change.
    /// Personal details count only in outside text: a result marked so, a summary of some, and
    /// the messages of a channel person not trusted (`person_outside`).
    pub fn chat_hidden(
        &self,
        system: Option<&str>,
        messages: &[Message],
        person_outside: bool,
    ) -> Hidden {
        let mut hidden = Hidden::default();
        let l = self.learner();
        if !l.active() {
            return hidden;
        }
        l.learn(&mut hidden, system.unwrap_or_default(), false);
        for m in messages {
            match m {
                Message::User(p) => l.learn(&mut hidden, &p.for_model(), person_outside),
                Message::Summary { text, outside, .. } => {
                    l.learn(&mut hidden, text, outside.is_some());
                }
                Message::Assistant { text, tools, .. } => {
                    l.learn(&mut hidden, text, false);
                    for t in tools {
                        l.learn_json(&mut hidden, &t.args, false);
                        let output = t.output.as_deref().unwrap_or_default();
                        l.learn(&mut hidden, output, t.outside.is_some());
                    }
                }
                Message::Divider(_) => {}
            }
        }
        hidden
    }

    /// A reply as a channel person may see it: with "Check replies to channels" on, held back if it
    /// repeats `brief`, and with paths and local addresses left out (see [`replies`]).
    pub fn check_reply(&self, text: &str, brief: Option<&str>) -> String {
        if self.on(Pack::Replies) {
            replies::check(text, brief)
        } else {
            text.to_owned()
        }
    }

    /// Notes what to hide in a result that is outside text, for work that keeps its own [`Hidden`]
    /// (a team member).
    pub fn learn_outside(&self, hidden: &mut Hidden, text: &str) {
        self.learner().learn(hidden, text, true);
    }

    /// `req`, ready to send: characters a person cannot see taken out, every value to hide in it,
    /// known to `hidden` or found now (and added to it), replaced with its placeholder, and the
    /// model told what placeholders are.
    pub fn request(&self, mut req: Request, hidden: &mut Hidden) -> Guarded {
        let mut stripped = 0;
        if self.on(Pack::Injection) {
            for text in [&mut req.system, &mut req.context].into_iter().flatten() {
                stripped += injection::strip_in(text);
            }
            for t in &mut req.turns {
                stripped += injection::strip_in(&mut t.text);
            }
            for tool in &mut req.tools {
                stripped += injection::strip_in(&mut tool.description);
            }
        }
        let l = self.learner();
        if l.active() {
            l.learn(hidden, req.system.as_deref().unwrap_or_default(), false);
            l.learn(hidden, req.context.as_deref().unwrap_or_default(), false);
            for t in &req.turns {
                l.learn(hidden, &t.text, false);
                for c in &t.calls {
                    l.learn_json(hidden, &c.args, false);
                }
            }
        }
        if hidden.is_empty() {
            return Guarded { req, stripped };
        }
        let m = hidden.masker(hidden);
        req.system = req.system.map(|s| m.hide(&s));
        for t in &mut req.turns {
            t.text = m.hide(&t.text);
            for c in &mut t.calls {
                c.args = m.hide_json(&c.args);
            }
        }
        req.context = Some(match req.context.take() {
            Some(c) => format!("{}\n\n{PLACEHOLDER_NOTE}", m.hide(&c)),
            None => PLACEHOLDER_NOTE.to_owned(),
        });
        Guarded { req, stripped }
    }

    /// A tool call the model asked for, before it runs: its arguments with the hidden values put
    /// back, and the rule it breaks, if any (a refusal before a question). `outside` says what
    /// outside text the model had in front of it when it asked (see [`taint`]).
    pub fn review(
        &self,
        hidden: &Hidden,
        name: &str,
        args: &str,
        at: &Place,
        outside: Option<&str>,
    ) -> (String, Option<actions::Finding>) {
        let mut found = Vec::new();
        // Put back whatever was hidden, even if the pack was turned off since.
        let revealed = hidden.reveal_json(args);
        if self.on(Pack::Secrets) || !hidden.is_empty() {
            if let Some(unknown) = secrets::first_placeholder(&revealed) {
                found.push(actions::Finding {
                    rule: "secrets.unknown",
                    why: format!("It uses {unknown}, a hidden value this chat does not have."),
                    action: Action::Block,
                });
            } else if secrets::has_placeholder(args) && off_machine(name) {
                found.push(actions::Finding {
                    rule: "secrets.off_machine",
                    why: "It would send a hidden secret away from this computer.".to_owned(),
                    action: Action::Ask,
                });
            }
        }
        if self.on(Pack::Actions) {
            if name == crate::tools::SHELL
                && let Some(spec) = crate::command::parse(&revealed)
            {
                found.extend(actions::check_command(&spec.command, at));
            }
            if name == crate::files::WRITE || name == crate::files::EDIT {
                found.extend(actions::check_write(&crate::files::path_of(&revealed), at));
            }
        }
        if self.on(Pack::Taint)
            && let Some(from) = outside
            && taint::acts(name)
        {
            found.push(actions::Finding {
                rule: "taint.outside",
                why: format!(
                    "What it was reading holds {from}, which could be steering it. Check this is what you want."
                ),
                action: Action::Ask,
            });
        }
        found.sort_by_key(|f| f.action != Action::Block);
        (revealed, found.into_iter().next())
    }

    /// A reply going to a channel person: every hidden value in it (a secret, a personal detail
    /// from outside text, a blocked word) left as its placeholder but those they sent themselves
    /// (in `theirs`), numbered as the model saw them.
    pub fn for_channel<'t>(
        &self,
        text: &str,
        hidden: &Hidden,
        theirs: impl IntoIterator<Item = &'t str>,
    ) -> String {
        let l = self.learner();
        if hidden.is_empty() && !l.active() {
            return text.to_owned();
        }
        // Secrets and blocked words it wrote itself count too; personal details only from the vault.
        let mut all = hidden.clone();
        l.learn(&mut all, text, false);
        let mut own = Hidden::default();
        for t in theirs {
            l.learn(&mut own, t, true);
        }
        let hide = all.except(|v| own.contains(v));
        hide.masker(&all).hide(text)
    }
}

/// Whether a tool sends what it is given away from this computer, past the guard.
fn off_machine(name: &str) -> bool {
    name == crate::web::NAME
        || name == crate::channels::SEND_FILE
        || crate::mcp::is_tool(name)
        || crate::tools::needs_typesafe(name)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;
    use crate::completion::{Call, Turn};

    const KEY: &str = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123";

    fn at() -> Place {
        Place {
            root: PathBuf::from("/home/ada/project"),
            home: Some(PathBuf::from("/home/ada")),
        }
    }

    fn request(turns: Vec<Turn>) -> Request {
        Request {
            model: "m".into(),
            system: Some("You help.".into()),
            context: Some("It is noon.".into()),
            turns,
            think: None,
            tools: Vec::new(),
        }
    }

    #[test]
    fn a_request_leaves_with_every_secret_hidden_and_the_model_told_why() {
        let g = Guard::default();
        let mut hidden = Hidden::default();
        let args = json!({"path": ".env", "content": format!("KEY={KEY}")}).to_string();
        let sent = g.request(
            request(vec![
                Turn::user(format!("my key is {KEY}")),
                Turn::assistant(
                    "",
                    vec![Call {
                        id: "c1".into(),
                        name: "write_file".into(),
                        args,
                    }],
                ),
                Turn::tool("c1", "wrote .env"),
            ]),
            &mut hidden,
        );
        let r = sent.request();
        assert_eq!(r.turns[0].text, "my key is <<SECRET_1>>");
        assert!(r.turns[1].calls[0].args.contains("KEY=<<SECRET_1>>"));
        assert!(!format!("{r:?}").contains(KEY), "nothing carries the key");
        assert!(r.context.as_deref().unwrap().starts_with("It is noon."));
        assert!(r.context.as_deref().unwrap().contains("<<SECRET_1>>"));
        assert_eq!(
            r.system.as_deref(),
            Some("You help."),
            "the cached part is untouched"
        );
    }

    #[test]
    fn with_the_pack_off_or_nothing_secret_the_request_is_unchanged() {
        let plain = request(vec![Turn::user("hello")]);
        let g = Guard::default();
        assert_eq!(
            g.request(plain.clone(), &mut Hidden::default()).request(),
            &plain
        );
        let mut off = Guard::default();
        off.set(Pack::Secrets, false);
        let secret = request(vec![Turn::user(KEY)]);
        assert_eq!(
            off.request(secret.clone(), &mut Hidden::default())
                .request(),
            &secret
        );
    }

    #[test]
    fn a_call_gets_its_values_back_and_risky_ones_are_caught() {
        let g = Guard::default();
        let mut hidden = Hidden::default();
        hidden.learn(KEY);
        let write = json!({"path": ".env", "content": "KEY=<<SECRET_1>>"}).to_string();
        let (args, trip) = g.review(&hidden, "write_file", &write, &at(), None);
        assert!(args.contains(KEY) && trip.is_none(), "{args}");

        let fetch = json!({"url": "https://x.example/?k=<<SECRET_1>>"}).to_string();
        let (_, trip) = g.review(&hidden, crate::web::NAME, &fetch, &at(), None);
        assert_eq!(
            trip.map(|t| (t.rule, t.action)),
            Some(("secrets.off_machine", Action::Ask))
        );

        let unknown = json!({"command": "echo <<SECRET_7>>"}).to_string();
        let (_, trip) = g.review(&hidden, "shell", &unknown, &at(), None);
        assert_eq!(trip.map(|t| t.action), Some(Action::Block));

        let sudo = json!({"command": "sudo reboot"}).to_string();
        let (_, trip) = g.review(&hidden, "shell", &sudo, &at(), None);
        assert_eq!(trip.map(|t| t.rule), Some("shell.admin"));

        let mut off = Guard::default();
        off.set(Pack::Actions, false);
        assert!(off.review(&hidden, "shell", &sudo, &at(), None).1.is_none());
    }

    #[test]
    fn with_outside_text_in_view_a_call_that_acts_asks_and_one_that_reads_does_not() {
        let g = Guard::default();
        let hidden = Hidden::default();
        let write = json!({"path": "a.txt", "content": "x"}).to_string();
        let (_, trip) = g.review(&hidden, "write_file", &write, &at(), Some("a web page"));
        let trip = trip.unwrap();
        assert_eq!((trip.rule, trip.action), ("taint.outside", Action::Ask));
        assert!(trip.why.contains("a web page"), "{}", trip.why);
        let read = json!({"path": "a.txt"}).to_string();
        assert!(
            g.review(&hidden, "read_file", &read, &at(), Some("a web page"))
                .1
                .is_none()
        );
        assert!(
            g.review(&hidden, "write_file", &write, &at(), None)
                .1
                .is_none()
        );
        let mut off = Guard::default();
        off.set(Pack::Taint, false);
        assert!(
            off.review(&hidden, "write_file", &write, &at(), Some("a web page"))
                .1
                .is_none()
        );
    }

    #[test]
    fn a_channel_person_gets_back_only_the_secrets_they_sent() {
        let g = Guard::default();
        let mut hidden = Hidden::default();
        hidden.learn(&format!("{KEY} and password=hunter2hunter2"));
        let reply = format!("Saved {KEY} and hunter2hunter2.");
        assert_eq!(
            g.for_channel(&reply, &hidden, ["here: password=hunter2hunter2"]),
            "Saved <<SECRET_1>> and hunter2hunter2."
        );
        let invented = "Use ghp_abcdefghijklmnopqrstuvwxyz0123456789 now.";
        assert!(!g.for_channel(invented, &hidden, []).contains("ghp_"));
    }

    #[test]
    fn packs_start_as_agreed_and_switches_words_and_limits_are_saved() {
        let db = crate::testing::TempDb::new();
        let mut app = db.open();
        let on: Vec<Pack> = Pack::ALL.into_iter().filter(|p| app.guard.on(*p)).collect();
        assert_eq!(
            on,
            [Pack::Secrets, Pack::Actions, Pack::Taint, Pack::Injection],
            "personal details and reply checks start off"
        );
        app.set_pack(Pack::Actions, false);
        app.set_pack(Pack::Personal, true);
        app.set_guard_words("Falcon, acme\n\n falcon ");
        app.set_channel_limits(Limits {
            length: 500,
            per_hour: 10,
        });
        drop(app);
        let back = db.open();
        assert!(!back.guard.on(Pack::Actions) && back.guard.on(Pack::Personal));
        assert_eq!(
            back.guard.words(),
            ["Falcon", "acme"],
            "trimmed, repeats once"
        );
        assert_eq!(back.guard.limits().per_hour, 10);
    }
}

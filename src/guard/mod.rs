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
pub mod secrets;
pub mod taint;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use self::actions::Place;
pub use self::secrets::Hidden;
use crate::completion::Request;
use crate::model::Message;
use crate::store::Store;

/// Where the packs' switches are saved.
const PACKS: &str = "guard.packs";
/// Where how long a go-ahead is waited for is saved, in minutes.
const WAIT: &str = "guard.wait";

/// How long a call held for a go-ahead waits, in minutes, until it is changed on the Guardrails
/// screen; after it, the call does not run.
const DEFAULT_WAIT: u32 = 10;

/// The waits offered on the Guardrails screen, in minutes.
pub const WAITS: [u32; 4] = [5, 10, 30, 60];

/// What the model is told when values are hidden from it.
const PLACEHOLDER_NOTE: &str = "Some values, such as keys and passwords, are hidden from you as placeholders like <<SECRET_1>>. Where a value is needed, in a tool's arguments too, write its placeholder exactly as it is: it is replaced with the real value on the person's computer. Never guess or ask for a hidden value.";

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
}

impl Pack {
    pub const ALL: [Pack; 3] = [Pack::Secrets, Pack::Actions, Pack::Taint];

    pub fn title(self) -> &'static str {
        match self {
            Self::Secrets => "Hide secrets from models",
            Self::Actions => "Stop risky actions",
            Self::Taint => "Ask before acting on outside text",
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
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Secrets => "secrets",
            Self::Actions => "actions",
            Self::Taint => "taint",
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
pub struct Guarded(Request);

impl Guarded {
    pub fn request(&self) -> &Request {
        &self.0
    }
}

/// Which packs are on: the rules every check applies.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Guard {
    /// Packs switched from how they start; any not here is on.
    packs: BTreeMap<Pack, bool>,
    /// How long a go-ahead is waited for, in minutes, when changed from [`DEFAULT_WAIT`].
    wait: Option<u32>,
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
        let wait = store.and_then(|db| {
            db.setting::<u32>(WAIT).unwrap_or_else(|e| {
                eprintln!("oido: could not read how long to wait for a go-ahead: {e}");
                None
            })
        });
        Guard {
            packs: Pack::ALL
                .into_iter()
                .filter_map(|p| saved.get(p.key()).map(|on| (p, *on)))
                .collect(),
            wait: wait.filter(|w| *w > 0),
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
        self.packs.get(&pack).copied().unwrap_or(true)
    }

    pub fn set(&mut self, pack: Pack, on: bool) {
        self.packs.insert(pack, on);
    }

    /// The packs as saved.
    fn saved(&self) -> BTreeMap<&'static str, bool> {
        Pack::ALL.iter().map(|p| (p.key(), self.on(*p))).collect()
    }

    /// The secrets of a chat with brief `system`: those in the brief first, then each in the
    /// order the chat first has it, so the numbering of what was sent before does not change.
    pub fn chat_hidden(&self, system: Option<&str>, messages: &[Message]) -> Hidden {
        let mut hidden = Hidden::default();
        if !self.on(Pack::Secrets) {
            return hidden;
        }
        hidden.learn(system.unwrap_or_default());
        for m in messages {
            match m {
                Message::User(p) => hidden.learn(&p.for_model()),
                Message::Summary { text, .. } => hidden.learn(text),
                Message::Assistant { text, tools, .. } => {
                    hidden.learn(text);
                    for t in tools {
                        hidden.learn_json(&t.args);
                        hidden.learn(t.output.as_deref().unwrap_or_default());
                    }
                }
                Message::Divider(_) => {}
            }
        }
        hidden
    }

    /// `req`, ready to send: every secret in it, known to `hidden` or found now (and added to it),
    /// replaced with its placeholder, and the model told what placeholders are.
    pub fn request(&self, mut req: Request, hidden: &mut Hidden) -> Guarded {
        if !self.on(Pack::Secrets) {
            return Guarded(req);
        }
        hidden.learn(req.system.as_deref().unwrap_or_default());
        hidden.learn(req.context.as_deref().unwrap_or_default());
        for t in &req.turns {
            hidden.learn(&t.text);
            for c in &t.calls {
                hidden.learn_json(&c.args);
            }
        }
        if hidden.is_empty() {
            return Guarded(req);
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
        Guarded(req)
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
        if self.on(Pack::Secrets) {
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

    /// A reply going to a channel person: every secret in it hidden but those they sent
    /// themselves (in `theirs`), numbered as the model saw them.
    pub fn for_channel<'t>(
        &self,
        text: &str,
        hidden: &Hidden,
        theirs: impl IntoIterator<Item = &'t str>,
    ) -> String {
        if !self.on(Pack::Secrets) {
            return text.to_owned();
        }
        let mut all = hidden.clone();
        all.learn(text);
        let mut own = Hidden::default();
        for t in theirs {
            own.learn(t);
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
    fn packs_start_on_and_a_switch_is_saved() {
        let db = crate::testing::TempDb::new();
        let app = db.open();
        assert!(Pack::ALL.iter().all(|p| app.guard.on(*p)));
        let mut app = app;
        app.set_pack(Pack::Actions, false);
        drop(app);
        let back = db.open();
        assert!(!back.guard.on(Pack::Actions) && back.guard.on(Pack::Secrets));
    }
}

//! Channels: talking to agents from a messaging app (see `docs/channels.md`). This is the part
//! no channel owns: who is paired, which agent each person talks to, the chat they carry on with
//! it, and what goes back when a turn is over. Each channel (`telegram`) turns what it hears into
//! [`Heard`] and sends each [`Out`] its own way.
//!
//! A bot connected on the Channels screen is heard while the app runs. Only people paired with it
//! are answered: a stranger gets a code, and you approve it on the Channels screen. A paired person
//! talks to one agent at a time, from those chosen for that channel, and carries on one chat with
//! each: an ordinary chat in the app, marked as theirs, that `new` replaces. When a turn in such a
//! chat is over, what the agent wrote goes back to them, whoever wrote the message it answers,
//! there or here; no desktop notification is raised for it.

mod press;
mod send;
pub mod slack;
pub mod telegram;

pub use send::{NAME as SEND_FILE, spec as send_file_spec};

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use rune::TextEdit;

use crate::app::{App, now};
use crate::model::{Message, chat};
use crate::prompt::Prompt;
use crate::providers::ModelStatus;
use crate::runs::Status as RunStatus;

/// The commands every channel takes, and what each does.
pub const COMMANDS: [(&str, &str); 4] = [
    ("agent", "Choose the agent you talk to"),
    ("new", "Start a fresh chat with it"),
    ("stop", "Stop what it is doing"),
    ("help", "What this bot does"),
];

/// How long a pairing code is good for, in seconds.
const PAIRING_SECS: i64 = 600;
/// A message older than this when it is heard was sent while Oido was closed.
const STALE_SECS: i64 = 600;
/// The most people waiting to be paired at once; more are turned away until some expire.
const MAX_PENDING: usize = 10;

const OFFLINE: &str = "Oido was offline when you sent this. Send it again if you still need it.";
const NO_AGENTS: &str = "No agent is available here yet. In Oido, choose agents for this channel on the **Channels** screen.";

/// A messaging app agents can be reached through.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Telegram,
    Slack,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Telegram, Kind::Slack];

    /// Its name in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Telegram => "telegram",
            Kind::Slack => "slack",
        }
    }

    /// Its name in words.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Telegram => "Telegram",
            Kind::Slack => "Slack",
        }
    }

    /// How command `name` is written there.
    pub fn command(self, name: &str) -> String {
        match self {
            Kind::Telegram => format!("/{name}"),
            Kind::Slack => format!("/oido {name}"),
        }
    }
}

/// Where a message goes: a chat, and a thread in it on channels that have them. A person's
/// private chat is named by their id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    pub chat: String,
    pub thread: Option<String>,
}

impl Place {
    /// `user`'s private chat with the bot.
    pub fn private(user: &str) -> Place {
        Place {
            chat: user.to_owned(),
            thread: None,
        }
    }
}

/// Someone who wrote, as their channel knows them.
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub id: String,
    /// First and last name.
    pub name: String,
    /// What to greet them by.
    pub first_name: String,
    pub handle: Option<String>,
}

/// A picture someone sent: its name, and its bytes or why there are none.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub name: String,
    pub bytes: Result<Vec<u8>, String>,
}

/// What a channel heard, in its own terms turned into these.
#[derive(Clone, Debug, PartialEq)]
pub enum Heard {
    /// Words, a picture, or both, sent at `at` (seconds since the epoch). `id` is the message's
    /// own, where the channel has one: what a "working" mark goes on.
    Message {
        from: Person,
        place: Place,
        id: Option<String>,
        at: i64,
        text: String,
        picture: Option<Picture>,
    },
    /// One of [`COMMANDS`], however the channel writes it.
    Command {
        from: Person,
        place: Place,
        id: Option<String>,
        at: i64,
        name: String,
        arg: String,
    },
    /// A press on a button under one of the bot's messages: the press's id, what the button
    /// sends back, and the message it was under (where, and its id).
    Press {
        from: Person,
        id: String,
        data: String,
        message: Option<(Place, String)>,
    },
}

/// A button under a message: its label and what a press sends back.
#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub label: String,
    pub data: String,
}

/// Something to send, in order, from the channel's own thread.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    /// A message, written in Markdown, with buttons under it.
    Say {
        to: Place,
        text: String,
        buttons: Vec<Button>,
    },
    /// "typing…" for a few seconds, where the channel has it.
    Typing { to: Place },
    /// The agent took up message `message` (`on`), or is done with it: a mark on it, where the
    /// channel shows one.
    Working {
        to: Place,
        message: String,
        on: bool,
    },
    /// Takes the buttons off message `message`, so none is pressed twice.
    Unbutton { to: Place, message: String },
    /// Ends a button press's spinner, with a short note if there is one.
    Answered {
        callback: String,
        note: Option<String>,
    },
}

/// How a channel's link stands, as its listener reports it.
#[derive(Clone, Debug, PartialEq)]
pub enum Health {
    Listening,
    /// It cannot reach the channel for now and keeps trying.
    Retrying(String),
    /// It stopped for good: the credentials no longer work.
    Stopped(String),
}

/// What a channel signs in with, opened.
#[derive(Clone, Debug)]
pub enum Creds {
    Telegram(telegram::Token),
    Slack(slack::Tokens),
}

impl Creds {
    /// Reads `secret` as saved for `kind`.
    fn parse(kind: Kind, secret: &str) -> Option<Creds> {
        match kind {
            Kind::Telegram => telegram::Token::parse(secret).map(Creds::Telegram),
            Kind::Slack => slack::Tokens::from_secret(secret).map(Creds::Slack),
        }
    }
}

/// The people paired with a channel, by id, shared with its listener (it fetches only their
/// pictures).
pub type Allowlist = Arc<RwLock<HashSet<String>>>;

/// A running channel: its threads stop when this is dropped (one in flight ends on its own, and
/// what it brings back is ignored, as its generation no longer matches).
#[derive(Debug)]
pub struct Link {
    stop: Arc<AtomicBool>,
    outbox: Sender<Out>,
}

impl Link {
    pub fn new(stop: Arc<AtomicBool>, outbox: Sender<Out>) -> Link {
        Link { stop, outbox }
    }

    fn send(&self, out: Out) {
        let _ = self.outbox.send(out);
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Someone allowed to talk to agents through a channel.
#[derive(Clone, Debug, PartialEq)]
pub struct Paired {
    /// Their id on the channel, which also names their private chat with the bot.
    pub id: String,
    pub name: String,
    pub handle: Option<String>,
    /// The agent their next message goes to.
    pub agent: Option<String>,
}

/// The chat a paired person carries on with an agent.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub user: String,
    pub agent: String,
    pub session: String,
}

/// Someone who asked to be paired, waiting for you to approve their code.
#[derive(Clone, Debug, PartialEq)]
pub struct Pairing {
    pub user: String,
    pub name: String,
    pub handle: Option<String>,
    pub code: String,
    pub expires: i64,
}

/// Where a channel stands, in words for the Channels screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    Off,
    Connecting,
    Listening,
    /// It cannot reach the channel for now and keeps trying.
    Retrying(String),
    /// Connecting failed, or the credentials stopped working.
    Failed(String),
}

/// One channel: its bot, the people paired with it, their chats, and what is in flight.
#[derive(Debug, Default)]
pub struct Channel {
    /// The connected bot's handle; `None` when none is connected.
    pub bot: Option<String>,
    /// Whether the connected bot listens.
    pub enabled: bool,
    pub status: Status,
    pub users: Vec<Paired>,
    pub pending: Vec<Pairing>,
    pub chats: Vec<Binding>,
    /// The agents people here may pick, by id, chosen on the Channels screen.
    pub agents: Vec<String>,
    /// The Channels screen's picker for them.
    pub picker: crate::picker::MultiPicker,
    /// Disconnecting waits for a second press.
    pub confirm_disconnect: bool,
    /// How far each chat's messages have gone back.
    delivered: HashMap<String, usize>,
    /// Where each chat's person last wrote from, where its replies go.
    places: HashMap<String, Place>,
    /// The messages each chat's agent has taken up and not yet answered, marked as worked on.
    working: HashMap<String, Vec<(Place, String)>>,
    /// What each live button does, by the data it sends back.
    presses: HashMap<String, press::Press>,
    next_group: u64,
    /// Drafts already offered with buttons, by their key.
    offered: HashSet<String>,
    link: Option<Link>,
    /// What the bot listening signs in with, for sending files; `None` while it is not.
    creds: Option<Creds>,
    allowlist: Allowlist,
    /// Another API address than the channel's own, for tests.
    api: Option<String>,
    /// Counts links started, so what an old one hears or says is ignored.
    generation: u64,
    /// What was sent, in tests (no threads run there).
    #[cfg(test)]
    pub sent: Vec<Out>,
}

impl Channel {
    pub fn connected(&self) -> bool {
        self.bot.is_some()
    }

    /// Whether chat `sid` is one a paired person carries on here.
    fn binds(&self, sid: &str) -> bool {
        self.chats.iter().any(|b| b.session == sid)
    }

    fn send(&mut self, out: Out) {
        if let Some(link) = &self.link {
            link.send(out.clone());
        }
        #[cfg(test)]
        self.sent.push(out);
    }

    /// Sends `markdown` to `to`, with `buttons` under it.
    fn say_with(&mut self, to: &Place, markdown: &str, buttons: Vec<Button>) {
        if markdown.trim().is_empty() {
            return;
        }
        self.send(Out::Say {
            to: to.clone(),
            text: markdown.to_owned(),
            buttons,
        });
    }

    /// Sends `markdown` to `user` privately.
    fn say(&mut self, user: &str, markdown: &str) {
        self.say_with(&Place::private(user), markdown, Vec::new());
    }

    /// The person who carries chat `sid` on, if anyone does.
    fn user_of(&self, sid: &str) -> Option<String> {
        self.chats
            .iter()
            .find(|b| b.session == sid)
            .map(|b| b.user.clone())
    }

    fn user(&self, id: &str) -> Option<&Paired> {
        self.users.iter().find(|u| u.id == id)
    }

    /// Where chat `sid`'s replies go: where its person last wrote from, else privately.
    fn place_of(&self, sid: &str, user: &str) -> Place {
        self.places
            .get(sid)
            .cloned()
            .unwrap_or_else(|| Place::private(user))
    }

    /// Starts `kind`'s threads with `creds`. Nothing starts without a running app (tests,
    /// headless).
    fn start(&mut self, creds: Creds) {
        self.generation += 1;
        self.link = None;
        self.status = Status::Connecting;
        self.creds = Some(creds.clone());
        self.sync_allowlist();
        if let Some(app) = crate::app::handle() {
            let allowlist = Arc::clone(&self.allowlist);
            self.link = Some(match creds {
                Creds::Telegram(token) => {
                    telegram::start(app.clone(), token, allowlist, self.generation)
                }
                Creds::Slack(tokens) => {
                    slack::start(app.clone(), tokens, allowlist, self.generation)
                }
            });
        }
    }

    fn stop(&mut self) {
        self.generation += 1;
        self.link = None;
        self.creds = None;
        self.status = Status::Off;
    }

    /// Tells the listener who is paired now.
    fn sync_allowlist(&self) {
        if let Ok(mut ids) = self.allowlist.write() {
            *ids = self.users.iter().map(|u| u.id.clone()).collect();
        }
    }
}

/// Every channel, and the forms that connect them.
#[derive(Debug, Default)]
pub struct Channels {
    pub telegram: Channel,
    /// The Telegram bot token being typed on the Channels screen.
    pub telegram_token: TextEdit,
    pub slack: Channel,
    /// The Slack app's tokens being typed on the Channels screen: its app-level token (`xapp-`)
    /// and its bot token (`xoxb-`).
    pub slack_app_token: TextEdit,
    pub slack_bot_token: TextEdit,
}

impl Channels {
    pub fn get(&self, kind: Kind) -> &Channel {
        match kind {
            Kind::Telegram => &self.telegram,
            Kind::Slack => &self.slack,
        }
    }

    pub fn get_mut(&mut self, kind: Kind) -> &mut Channel {
        match kind {
            Kind::Telegram => &mut self.telegram,
            Kind::Slack => &mut self.slack,
        }
    }

    /// The channel whose paired person carries chat `sid` on, if any.
    pub fn kind_of(&self, sid: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| self.get(*k).binds(sid))
    }

    /// The channel chat `sid` can send files on: one whose person last wrote privately (a file
    /// never goes into a group's thread) on a channel that takes files from agents.
    pub fn files_go(&self, sid: &str) -> Option<Kind> {
        let kind = self.kind_of(sid)?;
        let c = self.get(kind);
        let user = c.user_of(sid)?;
        (send::takes_files(kind) && c.place_of(sid, &user).thread.is_none()).then_some(kind)
    }
}

/// A six-digit code, random.
fn pairing_code() -> String {
    let mut bytes = [0u8; 4];
    if getrandom::fill(&mut bytes).is_err() {
        // No randomness: a code nobody can guess cannot be made, and none is ever approved blind.
        return String::new();
    }
    format!("{:06}", u32::from_le_bytes(bytes) % 1_000_000)
}

impl App {
    /// Reads each channel's bot, the people paired and their chats, and starts listening where
    /// it should.
    pub(crate) fn load_channels(&mut self) {
        for kind in Kind::ALL {
            self.load_channel(kind);
        }
    }

    fn load_channel(&mut self, kind: Kind) {
        let Some(db) = &self.store else {
            return;
        };
        let what = kind.title();
        let c = self.channels.get_mut(kind);
        match db.channel_users(kind) {
            Ok(list) => c.users = list,
            Err(e) => eprintln!("oido: could not load {what}'s paired people: {e}"),
        }
        match db.channel_chats(kind) {
            Ok(list) => c.chats = list,
            Err(e) => eprintln!("oido: could not load {what}'s chats: {e}"),
        }
        match db.channel_agents(kind) {
            Ok(list) => c.agents = list,
            Err(e) => eprintln!("oido: could not load {what}'s agents: {e}"),
        }
        match db.channel_bot(kind) {
            Ok(Some(bot)) => {
                c.bot = Some(bot.name);
                c.enabled = bot.enabled;
                match Creds::parse(kind, &bot.secret) {
                    Some(creds) if bot.enabled => c.start(creds),
                    Some(_) => {}
                    None => c.status = Status::Failed("What it signs in with is not valid.".into()),
                }
            }
            Ok(None) => {}
            Err(e) => {
                c.status =
                    Status::Failed(format!("What it signs in with could not be opened: {e}."));
            }
        }
    }

    // ---- the Channels screen

    /// Saves `kind`'s bot `name` with `secret` sealed, then starts listening with `creds`.
    /// Says why on the screen, and changes nothing, if it cannot be saved.
    fn channel_connected(&mut self, kind: Kind, secret: &str, name: String, creds: Creds) -> bool {
        if let Some(db) = &self.store
            && let Err(e) = db.save_channel_bot(kind, secret, &name, true)
        {
            self.channels.get_mut(kind).status =
                Status::Failed(format!("It could not be saved: {e}."));
            return false;
        }
        let c = self.channels.get_mut(kind);
        c.bot = Some(name);
        c.enabled = true;
        c.start(creds);
        true
    }

    /// Turns listening on or off, keeping the bot.
    pub fn set_channel_enabled(&mut self, kind: Kind, on: bool) {
        if !self.channels.get(kind).connected() {
            return;
        }
        self.save("the bot", |db| db.set_channel_enabled(kind, on));
        let creds = on
            .then_some(self.store.as_ref())
            .flatten()
            .and_then(|db| db.channel_bot(kind).ok().flatten())
            .and_then(|bot| Creds::parse(kind, &bot.secret));
        let c = self.channels.get_mut(kind);
        c.enabled = on;
        match (on, creds) {
            (false, _) => c.stop(),
            (true, Some(creds)) => c.start(creds),
            (true, None) => {
                c.status = Status::Failed(
                    "What it signs in with could not be opened. Connect again.".into(),
                );
            }
        }
    }

    /// Stops listening and forgets the bot. Paired people stay paired for the next one.
    pub fn disconnect_channel(&mut self, kind: Kind) {
        self.save("the bot", |db| db.delete_channel_bot(kind));
        let c = self.channels.get_mut(kind);
        c.stop();
        c.bot = None;
        c.enabled = false;
        c.pending.clear();
        c.confirm_disconnect = false;
    }

    /// Pairs the person who asked with this code: from now on they are answered.
    pub fn approve_pairing(&mut self, kind: Kind, user: &str) {
        let c = self.channels.get_mut(kind);
        let Some(i) = c.pending.iter().position(|p| p.user == user) else {
            return;
        };
        let p = c.pending.remove(i);
        let available = self.channel_agents(kind);
        let paired = Paired {
            id: p.user.clone(),
            name: p.name,
            handle: p.handle,
            agent: match available.as_slice() {
                [(id, _)] => Some(id.clone()),
                _ => None,
            },
        };
        self.save("the paired person", |db| {
            db.save_channel_user(kind, &paired)
        });
        let c = self.channels.get_mut(kind);
        c.users.retain(|u| u.id != paired.id);
        c.users.push(paired);
        c.sync_allowlist();
        c.say(
            &p.user,
            "You are paired with Oido. Send a message to talk to an agent.",
        );
        self.offer_agents(kind, &p.user);
    }

    /// Turns down a request to pair.
    pub fn deny_pairing(&mut self, kind: Kind, user: &str) {
        self.channels
            .get_mut(kind)
            .pending
            .retain(|p| p.user != user);
    }

    /// Unpairs someone: they are ignored from now on. Their chats stay, unmarked.
    pub fn unpair(&mut self, kind: Kind, user: &str) {
        self.save("the paired person", |db| db.delete_channel_user(kind, user));
        let c = self.channels.get_mut(kind);
        c.users.retain(|u| u.id != user);
        c.chats.retain(|b| b.user != user);
        c.sync_allowlist();
    }

    /// The agents people on `kind` may pick, by id and name, in the sidebar's order. Only agents
    /// of their own: a team member is reached through its lead.
    pub(crate) fn channel_agents(&self, kind: Kind) -> Vec<(String, String)> {
        let chosen = &self.channels.get(kind).agents;
        self.roster
            .list
            .iter()
            .filter(|a| chosen.contains(&a.id))
            .map(|a| (a.id.clone(), a.name.clone()))
            .collect()
    }

    /// Lets people on `kind` pick agent `agent`, or no longer if they could.
    pub fn toggle_channel_agent(&mut self, kind: Kind, agent: &str) {
        let c = self.channels.get_mut(kind);
        let on = !c.agents.iter().any(|a| a == agent);
        if on {
            c.agents.push(agent.to_owned());
        } else {
            c.agents.retain(|a| a != agent);
        }
        self.save("the channel's agents", |db| {
            db.set_channel_agent(kind, agent, on)
        });
    }

    // ---- what the threads hand over

    pub(crate) fn channel_health(&mut self, kind: Kind, generation: u64, health: Health) {
        let c = self.channels.get_mut(kind);
        if generation != c.generation {
            return;
        }
        c.status = match health {
            Health::Listening => Status::Listening,
            Health::Retrying(why) => Status::Retrying(why),
            Health::Stopped(why) => {
                c.link = None;
                c.creds = None;
                Status::Failed(format!("Stopped: {why}. Connect again."))
            }
        };
    }

    pub(crate) fn channel_send_failed(&mut self, kind: Kind, generation: u64, why: String) {
        if generation == self.channels.get(kind).generation {
            eprintln!("oido: a {} message did not go: {why}", kind.title());
        }
    }

    /// Says "typing…" wherever a busy chat's replies go.
    pub(crate) fn channel_typing(&mut self, kind: Kind, generation: u64) {
        let c = self.channels.get(kind);
        if generation != c.generation {
            return;
        }
        let mut busy: Vec<Place> = c
            .chats
            .iter()
            .filter(|b| self.busy_in(&b.session))
            .map(|b| c.place_of(&b.session, &b.user))
            .collect();
        busy.dedup();
        let c = self.channels.get_mut(kind);
        for to in busy {
            c.send(Out::Typing { to });
        }
    }

    pub(crate) fn channel_heard(&mut self, kind: Kind, generation: u64, heard: Vec<Heard>) {
        if generation != self.channels.get(kind).generation {
            return;
        }
        let at = now();
        for h in heard {
            self.hear(kind, h, at);
        }
    }

    /// Acts on what `kind` heard, at time `at`.
    fn hear(&mut self, kind: Kind, heard: Heard, at: i64) {
        let (from, sent) = match &heard {
            Heard::Press { from, .. } => (from.clone(), None),
            Heard::Message { from, at, .. } | Heard::Command { from, at, .. } => {
                (from.clone(), Some(*at))
            }
        };
        if self.channels.get(kind).user(&from.id).is_none() {
            if sent.is_some() {
                self.ask_to_pair(kind, &from, at);
            }
            return;
        }
        if sent.is_some_and(|sent| at - sent > STALE_SECS) {
            self.channels.get_mut(kind).say(&from.id, OFFLINE);
            return;
        }
        match heard {
            Heard::Press {
                from,
                id,
                data,
                message,
            } => self.channel_press(kind, &from.id, id, &data, message),
            Heard::Command {
                from,
                place,
                name,
                arg,
                ..
            } => self.channel_command(kind, &from.id, &place, &name, arg.trim()),
            Heard::Message {
                from,
                place,
                id,
                text,
                picture,
                ..
            } => {
                let text = text.trim();
                let mut prompt = Prompt::from(text);
                if let Some(p) = picture {
                    let image = p
                        .bytes
                        .map_err(|why| format!("it could not be fetched: {why}"))
                        .and_then(|bytes| crate::prompt::Image::fit(&p.name, &bytes));
                    match image {
                        Ok(image) => prompt
                            .attachments
                            .push(crate::prompt::Attachment::Image(image)),
                        Err(why) => {
                            self.channels.get_mut(kind).say(
                                &from.id,
                                &format!("⚠️ The picture was not sent to the agent: {why}."),
                            );
                            return;
                        }
                    }
                }
                if prompt.is_empty() {
                    self.channels
                        .get_mut(kind)
                        .say(&from.id, "Only text and pictures can be read for now.");
                    return;
                }
                self.channel_ask(kind, &from.id, &place, id, prompt);
            }
        }
    }

    /// Someone not paired wrote: they get a code to have approved, and you are told.
    fn ask_to_pair(&mut self, kind: Kind, from: &Person, at: i64) {
        let c = self.channels.get_mut(kind);
        c.pending.retain(|p| p.expires > at);
        let code = match c.pending.iter().find(|p| p.user == from.id) {
            Some(p) => p.code.clone(),
            None if c.pending.len() >= MAX_PENDING => {
                c.say(
                    &from.id,
                    "Oido has too many requests waiting. Try again later.",
                );
                return;
            }
            None => {
                let code = pairing_code();
                if code.is_empty() {
                    return;
                }
                c.pending.push(Pairing {
                    user: from.id.clone(),
                    name: from.name.clone(),
                    handle: from.handle.clone(),
                    code: code.clone(),
                    expires: at + PAIRING_SECS,
                });
                crate::notify::post(crate::notify::Note {
                    title: kind.title().into(),
                    body: format!(
                        "{} asks to talk to your agents. Code {code}: approve it in Channels.",
                        from.name
                    ),
                    chat: None,
                });
                code
            }
        };
        c.say(
            &from.id,
            &format!(
                "Hi {}. To talk to agents here, the owner of this Oido approves code **{code}** in Channels. It is good for 10 minutes.",
                from.first_name
            ),
        );
    }

    fn channel_command(&mut self, kind: Kind, user: &str, place: &Place, name: &str, arg: &str) {
        match name {
            "agent" if arg.is_empty() => self.offer_agents(kind, user),
            "agent" => {
                let wanted = arg.to_lowercase();
                let agents = self.channel_agents(kind);
                let found = agents
                    .iter()
                    .find(|(_, n)| n.to_lowercase() == wanted)
                    .or_else(|| {
                        agents
                            .iter()
                            .find(|(_, n)| n.to_lowercase().starts_with(&wanted))
                    });
                match found {
                    Some((id, _)) => {
                        let id = id.clone();
                        self.pick_agent(kind, user, &id);
                    }
                    None => {
                        self.channels
                            .get_mut(kind)
                            .say(user, &format!("No agent here is called {arg}."));
                        self.offer_agents(kind, user);
                    }
                }
            }
            "new" => {
                let Some((agent, name)) = self.agent_for(kind, user) else {
                    self.offer_agents(kind, user);
                    return;
                };
                if let Some(sid) = self.open_channel_chat(kind, user, &agent) {
                    self.channels
                        .get_mut(kind)
                        .places
                        .insert(sid, place.clone());
                }
                self.channels
                    .get_mut(kind)
                    .say(user, &format!("A fresh chat with **{name}**."));
            }
            "stop" => {
                let sid = self.agent_for(kind, user).and_then(|(agent, _)| {
                    self.channels
                        .get(kind)
                        .chats
                        .iter()
                        .find(|b| b.user == user && b.agent == agent)
                        .map(|b| b.session.clone())
                });
                let Some(sid) =
                    sid.filter(|sid| self.busy_in(sid) || self.queued.contains_key(sid))
                else {
                    self.channels.get_mut(kind).say(user, "Nothing is running.");
                    return;
                };
                self.queued.remove(&sid);
                // A reply being written ends as stopped, and that goes back like any reply.
                let writing = self.live.contains_key(&sid);
                self.cancel_session(&sid);
                if !writing {
                    self.channels.get_mut(kind).say(user, "Stopped.");
                }
            }
            "start" | "help" => self.channel_help(kind, user),
            _ => {
                let what = format!("There is no {} command.", kind.command(name));
                self.channels.get_mut(kind).say(user, &what);
                self.channel_help(kind, user);
            }
        }
    }

    fn channel_help(&mut self, kind: Kind, user: &str) {
        let current = self
            .agent_for(kind, user)
            .map_or_else(String::new, |(_, name)| {
                format!("You are talking to **{name}**.\n\n")
            });
        let commands: Vec<String> = COMMANDS
            .iter()
            .map(|(c, what)| format!("{}: {what}", kind.command(c)))
            .collect();
        self.channels.get_mut(kind).say(
            user,
            &format!(
                "{current}Write to the agent as you would in Oido.\n\n{}",
                commands.join("\n")
            ),
        );
    }

    /// The agents to pick from, as buttons.
    fn offer_agents(&mut self, kind: Kind, user: &str) {
        let agents = self.channel_agents(kind);
        if agents.is_empty() {
            self.channels.get_mut(kind).say(user, NO_AGENTS);
            return;
        }
        let current = self.agent_for(kind, user).map(|(id, _)| id);
        let buttons = agents
            .into_iter()
            .map(|(id, name)| Button {
                label: if current.as_ref() == Some(&id) {
                    format!("✓ {name}")
                } else {
                    name
                },
                data: format!("agent:{id}"),
            })
            .collect();
        self.channels.get_mut(kind).say_with(
            &Place::private(user),
            "Who do you want to talk to?",
            buttons,
        );
    }

    fn channel_press(
        &mut self,
        kind: Kind,
        user: &str,
        callback: String,
        data: &str,
        message: Option<(Place, String)>,
    ) {
        let note = if let Some(id) = data.strip_prefix("agent:") {
            if self.channel_agents(kind).iter().any(|(a, _)| a == id) {
                let id = id.to_owned();
                self.pick_agent(kind, user, &id);
                None
            } else {
                Some("That agent is no longer available.".to_owned())
            }
        } else {
            self.press_button(kind, user, data, message)
        };
        self.channels
            .get_mut(kind)
            .send(Out::Answered { callback, note });
    }

    fn pick_agent(&mut self, kind: Kind, user: &str, agent: &str) {
        let Some(name) = self.roster.agent_ref(agent).map(|a| a.name.clone()) else {
            return;
        };
        let c = self.channels.get_mut(kind);
        let Some(p) = c.users.iter_mut().find(|u| u.id == user) else {
            return;
        };
        p.agent = Some(agent.to_owned());
        let p = p.clone();
        c.say(user, &format!("You are talking to **{name}** now."));
        self.save("the paired person", |db| db.save_channel_user(kind, &p));
    }

    /// The agent `user` talks to, by id and name: the one they picked if it is still available,
    /// else the only one there is.
    fn agent_for(&self, kind: Kind, user: &str) -> Option<(String, String)> {
        let agents = self.channel_agents(kind);
        let picked = self.channels.get(kind).user(user)?.agent.as_ref();
        picked
            .and_then(|id| agents.iter().find(|(a, _)| a == id))
            .or(match agents.as_slice() {
                [only] => Some(only),
                _ => None,
            })
            .cloned()
    }

    /// The chat `user` carries on with `agent`, made if there is none (or it was deleted).
    fn channel_chat(&mut self, kind: Kind, user: &str, agent: &str) -> Option<String> {
        let kept = self
            .channels
            .get(kind)
            .chats
            .iter()
            .find(|b| b.user == user && b.agent == agent)
            .map(|b| b.session.clone())
            .filter(|sid| self.roster.owner_of(sid).is_some_and(|a| a.id == agent));
        match kept {
            Some(sid) => Some(sid),
            None => self.open_channel_chat(kind, user, agent),
        }
    }

    /// Makes a new chat for `user` with `agent` and binds it, in place of the one before.
    fn open_channel_chat(&mut self, kind: Kind, user: &str, agent: &str) -> Option<String> {
        let name = self.channels.get(kind).user(user)?.name.clone();
        let session = chat(&format!("{} · {name}", kind.title()), Vec::new());
        let a = self.roster.agent_mut(agent)?;
        a.sessions.push(session.clone());
        let position = a.sessions.len() - 1;
        self.save("the chat", |db| db.save_session(agent, &session, position));
        let b = Binding {
            user: user.to_owned(),
            agent: agent.to_owned(),
            session: session.id.clone(),
        };
        self.save("the chat's binding", |db| db.save_channel_chat(kind, &b));
        let c = self.channels.get_mut(kind);
        c.chats
            .retain(|old| !(old.user == user && old.agent == agent));
        c.chats.push(b);
        Some(session.id)
    }

    /// A paired person wrote to their agent from `place`: it answers their question if one is
    /// waiting, else waits for the reply being written, else goes now.
    fn channel_ask(
        &mut self,
        kind: Kind,
        user: &str,
        place: &Place,
        id: Option<String>,
        prompt: Prompt,
    ) {
        let Some((agent, name)) = self.agent_for(kind, user) else {
            self.offer_agents(kind, user);
            return;
        };
        let model = self.roster.agent_ref(&agent).map(|a| a.model.clone());
        let refused = match model.map(|m| self.providers.status(m.as_ref())) {
            Some(ModelStatus::Ready(_)) => None,
            Some(ModelStatus::Unavailable { why, .. }) => {
                Some(format!("⚠️ {name} cannot answer: {why}"))
            }
            _ => Some(format!("⚠️ {name} has no model to answer with.")),
        };
        if let Some(why) = refused {
            self.channels.get_mut(kind).say(user, &why);
            return;
        }
        let Some(sid) = self.channel_chat(kind, user, &agent) else {
            return;
        };
        let c = self.channels.get_mut(kind);
        c.places.insert(sid.clone(), place.clone());
        // Taken up, whichever way it goes on: marked until the agent is done.
        if let Some(message) = id {
            c.working
                .entry(sid.clone())
                .or_default()
                .push((place.clone(), message.clone()));
            c.send(Out::Working {
                to: place.clone(),
                message,
                on: true,
            });
        }
        // A question waiting is answered in words; a picture waits until it is.
        if !self.busy_in(&sid)
            && prompt.attachments.is_empty()
            && let Some(call) = self.pending_call_in(&sid, crate::tools::ASK)
        {
            let text = prompt.text.trim();
            // "2" or "1, 3" picks options by their numbers.
            let answer = self
                .question_of(&sid, &call)
                .and_then(|q| press::picked(&q, text))
                .unwrap_or_else(|| text.to_owned());
            self.channels.get_mut(kind).forget_answers(&sid);
            self.resolve(&sid, &call, answer, true);
            return;
        }
        if self.busy_in(&sid) || self.awaiting_in(&sid) {
            let waits = if self.busy_in(&sid) {
                format!("{name} is still working. This goes next.")
            } else {
                format!("{name} waits for your answer first. This goes after it.")
            };
            match self.queued.get_mut(&sid) {
                Some(waiting) => waiting.append(prompt),
                None => {
                    self.queued.insert(sid, prompt);
                }
            }
            self.channels
                .get_mut(kind)
                .say_with(place, &waits, Vec::new());
            return;
        }
        self.post(&sid, prompt);
    }

    /// Chat `sid`'s turn is over (nothing more runs in it): if a paired person carries it on,
    /// what the agent wrote since their message goes back to where they wrote from, and its
    /// question and drafts to them privately. Returns whether it did, so no desktop
    /// notification is raised for it.
    pub(crate) fn channel_turn_over(
        &mut self,
        sid: &str,
        status: RunStatus,
        failure: Option<&str>,
    ) -> bool {
        let Some(kind) = self.channels.kind_of(sid) else {
            return false;
        };
        let Some(user) = self.channels.get(kind).user_of(sid) else {
            return false;
        };
        let Some(messages) = self
            .roster
            .owner_of(sid)
            .and_then(|a| a.sessions.iter().find(|s| s.id == sid))
            .map(|s| &s.messages)
        else {
            return false;
        };
        let after_you = messages
            .iter()
            .rposition(|m| matches!(m, Message::User(_)))
            .map_or(0, |i| i + 1);
        let from = self
            .channels
            .get(kind)
            .delivered
            .get(sid)
            .copied()
            .unwrap_or(after_you)
            .min(messages.len());
        let mut said: Vec<String> = Vec::new();
        for m in &messages[from..] {
            if let Message::Assistant { text, .. } = m
                && !text.trim().is_empty()
            {
                said.push(text.trim().to_owned());
            }
        }
        let asked = messages.last().and_then(|m| match m {
            Message::Assistant { tools, .. } => tools
                .iter()
                .find(|t| t.name == crate::tools::ASK && t.output.is_none())
                .map(|t| (t.id.clone(), crate::tools::question(&t.args))),
            _ => None,
        });
        let len = messages.len();
        // Told already (a turn can end twice when the run it started ended first): nothing new.
        if from == len && failure.is_none() && status != RunStatus::Cancelled {
            return true;
        }
        match (status, failure) {
            (RunStatus::Cancelled, _) => said.push("_Stopped._".into()),
            (_, Some(why)) => said.push(format!("⚠️ {why}")),
            _ => {}
        }
        let waiting = self.queued.contains_key(sid);
        let c = self.channels.get_mut(kind);
        c.delivered.insert(sid.to_owned(), len);
        // Done with what it took up, unless more waits to go next.
        if !waiting {
            for (to, message) in c.working.remove(sid).unwrap_or_default() {
                c.send(Out::Working {
                    to,
                    message,
                    on: false,
                });
            }
        }
        // What it wrote, then its question, then what it drafted: each with its own buttons.
        let offers = self.draft_offers(kind, sid, &user);
        if said.is_empty() && asked.is_none() && offers.is_empty() {
            said.push("Done.".into());
        }
        let c = self.channels.get_mut(kind);
        if !said.is_empty() {
            let to = c.place_of(sid, &user);
            c.say_with(&to, &said.join("\n\n"), Vec::new());
        }
        if let Some((call, q)) = asked {
            self.offer_question(kind, &user, sid, &call, &q);
        }
        let c = self.channels.get_mut(kind);
        for (text, buttons) in offers {
            c.say_with(&Place::private(&user), &text, buttons);
        }
        true
    }

    /// The channel chat `sid` is carried on from, if any, for the mark beside its name.
    pub fn on_channel(&self, sid: &str) -> Option<Kind> {
        self.channels.kind_of(sid)
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::Receiver;

    use rune::TextEdit;
    use serde_json::json;

    use super::*;
    use crate::testing::{EVENTS, TempDb, app_with_server, call_stream, text_stream};

    const T: Kind = Kind::Telegram;
    const ADA: &str = "42";

    fn from(id: &str) -> Person {
        Person {
            id: id.into(),
            name: "Ada".into(),
            first_name: "Ada".into(),
            handle: Some("ada".into()),
        }
    }

    /// What the channel hears when `id` writes `text` at `at`: a command if it starts with `/`.
    fn heard(id: &str, text: &str, at: i64) -> Heard {
        let place = Place::private(id);
        match text.strip_prefix('/') {
            Some(command) => {
                let (name, arg) = command.split_once(' ').unwrap_or((command, ""));
                Heard::Command {
                    from: from(id),
                    place,
                    id: None,
                    at,
                    name: name.into(),
                    arg: arg.into(),
                }
            }
            None => Heard::Message {
                from: from(id),
                place,
                id: None,
                at,
                text: text.into(),
                picture: None,
            },
        }
    }

    fn write(app: &mut App, text: &str) {
        let at = now();
        app.hear(T, heard(ADA, text, at), at);
    }

    /// What went out since last asked, as written, one entry per message.
    fn said(app: &mut App) -> Vec<String> {
        std::mem::take(&mut app.channels.telegram.sent)
            .into_iter()
            .filter_map(|o| match o {
                Out::Say { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    /// The sample agents with a server answering `replies`; the open agent alone on channels, a
    /// bot connected, and Ada paired.
    fn paired(db: &TempDb, replies: Vec<&'static str>) -> (App, Receiver<String>) {
        let (mut app, seen) =
            app_with_server(db, replies.into_iter().map(|r| (200, EVENTS, r)).collect());
        let open = app.current().unwrap().id.clone();
        app.channels.telegram.agents = vec![open];
        app.channels.telegram.bot = Some("oido_bot".into());
        write(&mut app, "hello?");
        app.approve_pairing(T, ADA);
        app.channels.telegram.sent.clear();
        (app, seen)
    }

    fn bound(app: &App) -> String {
        app.channels.telegram.chats[0].session.clone()
    }

    fn messages<'a>(app: &'a App, sid: &str) -> &'a [Message] {
        &app.roster
            .owner_of(sid)
            .unwrap()
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .unwrap()
            .messages
    }

    #[test]
    fn a_stranger_gets_a_code_and_is_answered_only_once_you_approve_it() {
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![]);
        app.channels.telegram.bot = Some("oido_bot".into());
        let chats_before = app
            .roster
            .list
            .iter()
            .map(|a| a.sessions.len())
            .sum::<usize>();
        write(&mut app, "hi");
        write(&mut app, "hello??");
        assert_eq!(
            app.channels.telegram.pending.len(),
            1,
            "one request however often they write"
        );
        let code = app.channels.telegram.pending[0].code.clone();
        assert_eq!(code.len(), 6);
        let told = said(&mut app);
        assert_eq!(told.len(), 2);
        assert!(told.iter().all(|t| t.contains(&code)), "{told:?}");
        assert_eq!(
            app.roster
                .list
                .iter()
                .map(|a| a.sessions.len())
                .sum::<usize>(),
            chats_before,
            "nothing reached an agent"
        );

        app.deny_pairing(T, ADA);
        assert!(app.channels.telegram.pending.is_empty() && app.channels.telegram.users.is_empty());
        write(&mut app, "please");
        app.approve_pairing(T, ADA);
        assert_eq!(app.channels.telegram.users.len(), 1);
        assert!(said(&mut app).iter().any(|t| t.contains("paired")));
        let back = db.open();
        assert_eq!(
            back.channels.telegram.users.len(),
            1,
            "the pairing is saved"
        );
    }

    #[test]
    fn messages_sent_while_oido_was_closed_are_not_answered() {
        let db = TempDb::new();
        let (mut app, _) = paired(&db, vec![]);
        let at = now();
        app.hear(T, heard(ADA, "old news", at - 3600), at);
        assert_eq!(said(&mut app), vec![OFFLINE.to_owned()]);
        assert!(
            app.channels.telegram.chats.is_empty(),
            "no chat was opened for it"
        );
    }

    #[test]
    fn a_message_goes_to_the_agent_in_a_chat_of_its_own_and_the_reply_goes_back() {
        let db = TempDb::new();
        let (mut app, seen) = paired(
            &db,
            vec![text_stream("Hello **Ada**"), text_stream("Again")],
        );
        write(&mut app, "hi there");
        assert!(seen.recv().unwrap().contains("hi there"));
        let sid = bound(&app);
        let agent = app.roster.owner_of(&sid).unwrap();
        assert_eq!(
            app.channels.telegram.agents,
            std::slice::from_ref(&agent.id)
        );
        let chat = agent.sessions.iter().find(|s| s.id == sid).unwrap();
        assert_eq!(chat.title, "Telegram · Ada");
        assert_eq!(app.on_channel(&sid), Some(T));
        let sent = std::mem::take(&mut app.channels.telegram.sent);
        assert!(
            sent.contains(&Out::Say {
                to: Place::private(ADA),
                text: "Hello **Ada**".into(),
                buttons: Vec::new(),
            }),
            "{sent:?}"
        );

        write(&mut app, "more");
        assert_eq!(bound(&app), sid, "the same chat carries on");
        assert_eq!(said(&mut app), vec!["Again".to_owned()]);
        assert_eq!(messages(&app, &sid).len(), 4);

        let back = db.open();
        assert_eq!(
            back.on_channel(&sid),
            Some(T),
            "the chat is still theirs after a restart"
        );
    }

    #[test]
    fn a_reply_to_a_message_typed_in_the_app_goes_to_telegram_too() {
        let db = TempDb::new();
        let (mut app, _) = paired(&db, vec![text_stream("one"), text_stream("two")]);
        write(&mut app, "hi");
        let sid = bound(&app);
        said(&mut app);
        app.open_chat(&sid);
        app.composer.edit = TextEdit::new("from the desk");
        app.send();
        assert_eq!(said(&mut app), vec!["two".to_owned()]);
    }

    #[test]
    fn the_agents_question_goes_to_telegram_and_the_next_message_answers_it() {
        let db = TempDb::new();
        let (mut app, seen) = paired(
            &db,
            vec![
                call_stream(
                    crate::tools::ASK,
                    json!({"question": "Which colour?", "options": ["Red", "Blue"]}),
                ),
                text_stream("Blue it is"),
            ],
        );
        write(&mut app, "paint it");
        let asked = said(&mut app);
        assert_eq!(asked.len(), 1);
        assert!(
            asked[0].contains("Which colour?") && asked[0].contains("2. Blue"),
            "{asked:?}"
        );
        write(&mut app, "Blue");
        let _ = seen.recv();
        assert!(
            seen.recv().unwrap().contains("blue"),
            "the answer went to the model"
        );
        assert_eq!(said(&mut app), vec!["Blue it is".to_owned()]);
    }

    #[test]
    fn a_message_while_the_agent_works_waits_and_stop_drops_it() {
        let db = TempDb::new();
        let (mut app, _) = paired(&db, vec![text_stream("ok")]);
        write(&mut app, "first");
        let sid = bound(&app);
        said(&mut app);
        // A command still running keeps the chat busy.
        app.commands
            .insert(sid.clone(), Arc::new(AtomicBool::new(false)));
        write(&mut app, "second");
        assert_eq!(
            app.queued.get(&sid).map(|p| p.text.as_str()),
            Some("second")
        );
        assert!(said(&mut app)[0].contains("still working"));
        write(&mut app, "/stop");
        assert!(app.queued.is_empty(), "the waiting message is dropped");
        assert_eq!(said(&mut app), vec!["Stopped.".to_owned()]);
        app.commands.clear();
        write(&mut app, "/stop");
        assert_eq!(said(&mut app), vec!["Nothing is running.".to_owned()]);
    }

    #[test]
    fn with_several_agents_one_is_picked_by_button_or_name_and_new_starts_fresh() {
        let db = TempDb::new();
        let (mut app, _) = paired(&db, vec![]);
        app.channels.telegram.agents = app.roster.list.iter().map(|a| a.id.clone()).collect();
        // Paired while one agent was available, they were given it; that choice is undone.
        app.channels.telegram.users[0].agent = None;
        let names: Vec<(String, String)> = app.channel_agents(T);
        assert!(names.len() > 1);
        write(&mut app, "hi");
        let offered = std::mem::take(&mut app.channels.telegram.sent);
        let Some(Out::Say { buttons, .. }) = offered.last() else {
            panic!("a choice: {offered:?}");
        };
        assert_eq!(buttons.len(), names.len());
        assert!(
            app.channels.telegram.chats.is_empty(),
            "nothing went until one is picked"
        );

        let (second, second_name) = names[1].clone();
        app.hear(
            T,
            Heard::Press {
                from: from(ADA),
                id: "cb".into(),
                data: format!("agent:{second}"),
                message: None,
            },
            now(),
        );
        assert_eq!(
            app.channels.telegram.users[0].agent.as_deref(),
            Some(second.as_str())
        );
        let sent = std::mem::take(&mut app.channels.telegram.sent);
        assert!(
            sent.iter()
                .any(|o| matches!(o, Out::Answered { callback, note: None } if callback == "cb"))
        );

        let (first, first_name) = names[0].clone();
        write(
            &mut app,
            &format!("/agent {}", first_name[..2].to_lowercase()),
        );
        assert_eq!(
            app.channels.telegram.users[0].agent.as_deref(),
            Some(first.as_str())
        );
        assert!(said(&mut app)[0].contains(&first_name));
        assert_ne!(first_name, second_name);

        write(&mut app, "/new");
        let one = bound(&app);
        write(&mut app, "/new");
        assert_eq!(app.channels.telegram.chats.len(), 1);
        assert_ne!(bound(&app), one, "a fresh chat replaces the old one");
        assert_eq!(app.on_channel(&one), None);
    }

    #[test]
    fn an_agent_switched_off_for_channels_is_not_reached() {
        let db = TempDb::new();
        let (mut app, _) = paired(&db, vec![]);
        app.channels.telegram.agents.clear();
        write(&mut app, "hi");
        assert_eq!(said(&mut app), vec![NO_AGENTS.to_owned()]);
        assert!(app.channels.telegram.chats.is_empty());
    }

    #[test]
    fn unpairing_ignores_them_and_leaves_their_chat_unmarked() {
        let db = TempDb::new();
        let (mut app, _) = paired(&db, vec![text_stream("ok")]);
        write(&mut app, "hi");
        let sid = bound(&app);
        app.unpair(T, ADA);
        assert_eq!(app.on_channel(&sid), None);
        assert!(app.roster.owner_of(&sid).is_some(), "the chat stays");
        said(&mut app);
        write(&mut app, "still there?");
        assert_eq!(
            app.channels.telegram.pending.len(),
            1,
            "they are a stranger again"
        );
    }

    /// The buttons of the last message sent, and forgets what was sent.
    fn last_buttons(app: &mut App) -> Vec<Button> {
        let sent = std::mem::take(&mut app.channels.telegram.sent);
        sent.iter()
            .rev()
            .find_map(|o| match o {
                Out::Say { buttons, .. } if !buttons.is_empty() => Some(buttons.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn press(app: &mut App, id: &str, data: &str) {
        app.hear(
            T,
            Heard::Press {
                from: from(id),
                id: "cb".into(),
                data: data.into(),
                message: Some((Place::private(id), "77".into())),
            },
            now(),
        );
    }

    fn note(app: &App) -> Option<String> {
        app.channels
            .telegram
            .sent
            .iter()
            .rev()
            .find_map(|o| match o {
                Out::Answered { note, .. } => Some(note.clone().unwrap_or_default()),
                _ => None,
            })
    }

    fn ask_stream(multiple: bool) -> &'static str {
        call_stream(
            crate::tools::ASK,
            json!({"question": "Which colour?", "options": ["Red", "Blue"], "multiple": multiple}),
        )
    }

    #[test]
    fn a_question_is_answered_with_a_button_once() {
        let db = TempDb::new();
        let (mut app, seen) = paired(&db, vec![ask_stream(false), text_stream("Blue it is")]);
        write(&mut app, "paint it");
        let buttons = last_buttons(&mut app);
        assert_eq!(
            buttons.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
            ["Red", "Blue"]
        );
        let _ = seen.recv();
        press(&mut app, ADA, &buttons[1].data);
        assert!(
            seen.recv().unwrap().contains("blue"),
            "the answer went to the model"
        );
        assert!(app.channels.telegram.sent.contains(&Out::Unbutton {
            to: Place::private(ADA),
            message: "77".into()
        }));
        assert_eq!(note(&app).as_deref(), Some("Blue"));
        assert_eq!(said(&mut app), vec!["Blue it is".to_owned()]);
        press(&mut app, ADA, &buttons[0].data);
        assert_eq!(note(&app).as_deref(), Some("This button no longer works."));
    }

    #[test]
    fn numbers_written_back_answer_a_question_and_its_buttons_then_stop_working() {
        let db = TempDb::new();
        let (mut app, seen) = paired(&db, vec![ask_stream(true), text_stream("Both")]);
        write(&mut app, "paint it");
        let asked = std::mem::take(&mut app.channels.telegram.sent);
        let Some(Out::Say { text, buttons, .. }) = asked.last() else {
            panic!("{asked:?}");
        };
        assert!(
            buttons.is_empty(),
            "several can be picked: they are written"
        );
        assert!(text.contains("numbers of your choices"));
        let _ = seen.recv();
        write(&mut app, "1, 2");
        assert!(seen.recv().unwrap().contains("red; blue"));
        assert!(app.channels.telegram.presses.is_empty());
    }

    #[test]
    fn someone_elses_button_does_nothing() {
        let db = TempDb::new();
        let (mut app, _) = paired(&db, vec![ask_stream(false)]);
        write(&mut app, "paint it");
        let buttons = last_buttons(&mut app);
        app.channels.telegram.users.push(Paired {
            id: "7".into(),
            name: "Eve".into(),
            handle: None,
            agent: None,
        });
        press(&mut app, "7", &buttons[0].data);
        assert_eq!(note(&app).as_deref(), Some("This button no longer works."));
        let sid = bound(&app);
        assert!(
            app.pending_call_in(&sid, crate::tools::ASK).is_some(),
            "still asked"
        );
    }

    #[test]
    fn a_drafted_agent_is_offered_and_created_with_a_press() {
        let db = TempDb::new();
        let (mut app, _) = paired(
            &db,
            vec![
                call_stream(
                    crate::agent_tool::NAME,
                    json!({"action": "create", "name": "Scribe", "instructions": "Write notes."}),
                ),
                text_stream("I drafted Scribe for you."),
            ],
        );
        write(&mut app, "make me a note taker");
        let sent = std::mem::take(&mut app.channels.telegram.sent);
        let texts: Vec<(String, Vec<Button>)> = sent
            .into_iter()
            .filter_map(|o| match o {
                Out::Say { text, buttons, .. } => Some((text, buttons)),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 2, "{texts:?}");
        assert_eq!(texts[0].0, "I drafted Scribe for you.");
        assert!(
            texts[1].0.starts_with("**New agent: Scribe**"),
            "{}",
            texts[1].0
        );
        let labels: Vec<&str> = texts[1].1.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["Create", "Reject"]);
        press(&mut app, ADA, &texts[1].1[0].data);
        assert!(app.roster.list.iter().any(|a| a.name == "Scribe"));
        let scribe = &app
            .roster
            .list
            .iter()
            .find(|a| a.name == "Scribe")
            .unwrap()
            .id;
        assert!(
            !app.channels.telegram.agents.contains(scribe),
            "an agent made from a channel is not put on it"
        );
        assert_eq!(said(&mut app), vec!["Done: New agent: Scribe.".to_owned()]);
    }

    #[test]
    fn a_drafted_schedule_can_be_rejected_with_a_press() {
        let db = TempDb::new();
        let (mut app, _) = paired(
            &db,
            vec![
                call_stream(
                    crate::schedule::tool::NAME,
                    json!({"action": "create", "name": "Standup", "prompt": "Plan my day", "every_minutes": 30}),
                ),
                text_stream("Drafted."),
            ],
        );
        write(&mut app, "remind me");
        let buttons = last_buttons(&mut app);
        assert_eq!(buttons.len(), 2);
        assert_eq!(app.proposals.len(), 1);
        press(&mut app, ADA, &buttons[1].data);
        assert!(app.proposals.is_empty() && app.schedules.is_empty());
        assert_eq!(
            said(&mut app),
            vec!["Rejected the schedule Standup.".to_owned()]
        );
    }

    #[test]
    fn a_learned_workflow_is_offered_when_ready_and_saved_with_a_press() {
        let db = TempDb::new();
        let (mut app, _) = paired(&db, vec![text_stream("ok")]);
        write(&mut app, "hi");
        let sid = bound(&app);
        said(&mut app);
        let agent_id = app.roster.owner_of(&sid).unwrap().id.clone();
        app.drafts.insert(
            sid.clone(),
            crate::workflow::Draft {
                session_id: sid.clone(),
                agent_id,
                name: "Morning brief".into(),
                goal: "Sum up the news.".into(),
                definition: Default::default(),
            },
        );
        app.channel_workflow_ready(&sid, None);
        let buttons = last_buttons(&mut app);
        assert_eq!(
            buttons.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(),
            ["Save", "Discard"]
        );
        press(&mut app, ADA, &buttons[0].data);
        assert!(app.workflows.iter().any(|w| w.name == "Morning brief"));
        assert!(!app.drafts.contains_key(&sid));
        assert_eq!(
            said(&mut app),
            vec!["Saved the workflow **Morning brief**.".to_owned()]
        );
    }

    fn png() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(2, 2)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    /// Ada sends a picture, as fetched (or not), with `caption`.
    fn picture(app: &mut App, bytes: Result<Vec<u8>, String>, caption: &str) {
        let at = now();
        let heard = Heard::Message {
            from: from(ADA),
            place: Place::private(ADA),
            id: None,
            at,
            text: caption.into(),
            picture: Some(Picture {
                name: "telegram-photo-1".into(),
                bytes,
            }),
        };
        app.hear(T, heard, at);
    }

    #[test]
    fn a_picture_goes_to_the_agent_with_its_caption() {
        let db = TempDb::new();
        let (mut app, seen) = paired(&db, vec![text_stream("A tiny square.")]);
        picture(&mut app, Ok(png()), "what is this?");
        let request = seen.recv().unwrap();
        assert!(
            request.contains("what is this?") && request.contains("image"),
            "{request}"
        );
        let sid = bound(&app);
        let Message::User(p) = &messages(&app, &sid)[0] else {
            panic!("the person's message");
        };
        assert_eq!(p.images().count(), 1);
        assert_eq!(said(&mut app), vec!["A tiny square.".to_owned()]);
    }

    #[test]
    fn a_picture_that_could_not_be_had_is_said_and_other_messages_are_turned_away() {
        let db = TempDb::new();
        let (mut app, _) = paired(&db, vec![]);
        picture(&mut app, Err("it is bigger than 10 MB".into()), "");
        let told = said(&mut app);
        assert!(told[0].contains("bigger than 10 MB"), "{told:?}");
        picture(&mut app, Ok(b"not a picture".to_vec()), "");
        assert!(said(&mut app)[0].contains("cannot be sent"));
        // A sticker, say: nothing the core can read.
        write(&mut app, "");
        assert_eq!(
            said(&mut app),
            vec!["Only text and pictures can be read for now.".to_owned()]
        );
        assert!(app.channels.telegram.chats.is_empty());
    }

    #[test]
    fn a_picture_sent_while_a_question_waits_goes_after_the_answer() {
        let db = TempDb::new();
        let (mut app, _) = paired(&db, vec![ask_stream(false)]);
        write(&mut app, "paint it");
        said(&mut app);
        picture(&mut app, Ok(png()), "");
        let sid = bound(&app);
        assert_eq!(app.queued.get(&sid).map(|p| p.images().count()), Some(1));
        assert!(said(&mut app)[0].contains("waits for your answer first"));
        assert!(app.pending_call_in(&sid, crate::tools::ASK).is_some());
    }

    fn folder_with(name: &str, body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("oido-send-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
        dir
    }

    #[test]
    fn an_agent_on_telegram_can_send_a_file_from_its_folder_and_nowhere_else() {
        let db = TempDb::new();
        let (mut app, seen) = paired(
            &db,
            vec![
                text_stream("hello"),
                call_stream(SEND_FILE, json!({"path": "report.txt", "caption": "Here"})),
                text_stream("Sent."),
                call_stream(SEND_FILE, json!({"path": "../secret"})),
                text_stream("Could not."),
            ],
        );
        let (tg, uploads) = crate::testing::serve(vec![(200, r#"{"ok":true,"result":{}}"#)]);
        app.channels.telegram.api = Some(tg);
        app.channels.telegram.creds =
            telegram::Token::parse("123:AAHabcdefghijklmnopqrstuvw").map(Creds::Telegram);
        let dir = folder_with("report.txt", "quarterly numbers");
        write(&mut app, "hi");
        let sid = bound(&app);
        app.roster.owner_of_mut(&sid).unwrap().workdir = Some(dir.display().to_string());
        assert!(
            seen.recv().unwrap().contains("\"send_file\""),
            "offered in a Telegram chat"
        );

        write(&mut app, "send me the report");
        let _ = seen.recv();
        let upload = uploads
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        assert!(
            upload.contains("/senddocument ") && upload.contains("quarterly numbers"),
            "{upload}"
        );
        assert!(
            upload.contains(&format!("\r\n\r\n{ADA}\r\n")),
            "to Ada: {upload}"
        );
        assert!(
            seen.recv()
                .unwrap()
                .contains("sent report.txt to them on telegram")
        );

        write(&mut app, "and the secret");
        let _ = seen.recv();
        assert!(seen.recv().unwrap().contains("outside the working folder"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn send_file_is_not_offered_outside_telegram_chats() {
        let db = TempDb::new();
        let (mut app, seen) = app_with_server(&db, vec![(200, EVENTS, text_stream("hi"))]);
        app.composer.edit = TextEdit::new("hello");
        app.send();
        assert!(!seen.recv().unwrap().contains("\"send_file\""));
    }

    const S: Kind = Kind::Slack;

    /// What Slack hears when Ada writes `text` at `place`, as message `id`.
    fn slack_heard(text: &str, place: Place, id: &str) -> Heard {
        Heard::Message {
            from: from("U1"),
            place,
            id: Some(id.into()),
            at: now(),
            text: text.into(),
            picture: None,
        }
    }

    fn thread() -> Place {
        Place {
            chat: "C1".into(),
            thread: Some("5.0".into()),
        }
    }

    /// Like [`paired`], on Slack: Ada is `U1`.
    fn slack_paired(db: &TempDb, replies: Vec<&'static str>) -> (App, Receiver<String>) {
        let (mut app, seen) =
            app_with_server(db, replies.into_iter().map(|r| (200, EVENTS, r)).collect());
        let open = app.current().unwrap().id.clone();
        app.channels.slack.agents = vec![open];
        app.channels.slack.bot = Some("oido in Acme".into());
        app.hear(S, slack_heard("hello?", thread(), "1.0"), now());
        app.approve_pairing(S, "U1");
        app.channels.slack.sent.clear();
        (app, seen)
    }

    #[test]
    fn a_stranger_mentioning_the_bot_gets_the_code_privately_not_in_the_thread() {
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![]);
        app.channels.slack.bot = Some("oido".into());
        app.hear(S, slack_heard("hi", thread(), "1.0"), now());
        let sent = std::mem::take(&mut app.channels.slack.sent);
        assert!(
            matches!(&sent[..], [Out::Say { to, text, .. }] if *to == Place::private("U1") && text.contains("code")),
            "{sent:?}"
        );
    }

    #[test]
    fn a_mention_is_answered_in_its_thread_and_the_question_goes_privately() {
        let db = TempDb::new();
        let (mut app, _) = slack_paired(
            &db,
            vec![
                call_stream(
                    crate::tools::ASK,
                    json!({"question": "Which colour?", "options": ["Red", "Blue"]}),
                ),
                text_stream("Blue it is"),
            ],
        );
        app.hear(S, slack_heard("paint it", thread(), "6.0"), now());
        let sid = app.channels.slack.chats[0].session.clone();
        assert_eq!(app.on_channel(&sid), Some(S));
        assert_eq!(app.channels.files_go(&sid), None, "no files into a thread");
        let sent = std::mem::take(&mut app.channels.slack.sent);
        assert!(sent.contains(&Out::Working {
            to: thread(),
            message: "6.0".into(),
            on: true
        }));
        let asked = sent.iter().find_map(|o| match o {
            Out::Say { to, text, buttons } if text.contains("Which colour?") => {
                Some((to.clone(), buttons.len()))
            }
            _ => None,
        });
        assert_eq!(
            asked,
            Some((Place::private("U1"), 2)),
            "asked privately, with buttons"
        );
        assert!(
            sent.contains(&Out::Working {
                to: thread(),
                message: "6.0".into(),
                on: false
            }),
            "asking ends the turn: the mark comes off"
        );

        app.hear(S, slack_heard("Blue", thread(), "7.0"), now());
        let sent = std::mem::take(&mut app.channels.slack.sent);
        assert!(
            sent.contains(&Out::Say {
                to: thread(),
                text: "Blue it is".into(),
                buttons: Vec::new()
            }),
            "the answer goes to the thread: {sent:?}"
        );
        assert!(
            sent.contains(&Out::Working {
                to: thread(),
                message: "7.0".into(),
                on: false
            }),
            "the answer's mark comes off: {sent:?}"
        );
    }

    #[test]
    fn a_direct_message_can_send_files_and_help_names_oido() {
        let db = TempDb::new();
        let (mut app, _) = slack_paired(&db, vec![text_stream("hi")]);
        let dm = Place {
            chat: "D9".into(),
            thread: None,
        };
        app.hear(S, slack_heard("hello", dm, "8.0"), now());
        let sid = app.channels.slack.chats[0].session.clone();
        assert_eq!(
            app.channels.files_go(&sid),
            Some(S),
            "files go in a direct message"
        );
        app.channels.slack.sent.clear();
        app.hear(
            S,
            Heard::Command {
                from: from("U1"),
                place: Place::private("U1"),
                id: None,
                at: now(),
                name: "help".into(),
                arg: String::new(),
            },
            now(),
        );
        let told = said_on(&mut app, S);
        assert!(told[0].contains("/oido agent: Choose"), "{told:?}");
    }

    #[test]
    fn a_mark_stays_while_a_message_waits_to_go_next() {
        let db = TempDb::new();
        let (mut app, _) = slack_paired(&db, vec![text_stream("one"), text_stream("two")]);
        app.hear(S, slack_heard("first", thread(), "6.0"), now());
        let sid = app.channels.slack.chats[0].session.clone();
        app.channels.slack.sent.clear();
        app.commands
            .insert(sid.clone(), Arc::new(AtomicBool::new(false)));
        app.hear(S, slack_heard("second", thread(), "7.0"), now());
        assert!(app.queued.contains_key(&sid));
        let marks = app.channels.slack.working.get(&sid).map(Vec::len);
        assert_eq!(marks, Some(1), "the second is marked while it waits");
    }

    /// What went out on `kind` since last asked.
    fn said_on(app: &mut App, kind: Kind) -> Vec<String> {
        std::mem::take(&mut app.channels.get_mut(kind).sent)
            .into_iter()
            .filter_map(|o| match o {
                Out::Say { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn agents_are_chosen_per_channel_and_kept() {
        let db = TempDb::new();
        let (mut app, _) = app_with_server(&db, vec![]);
        let (a, b) = (app.roster.list[0].id.clone(), app.roster.list[1].id.clone());
        app.toggle_channel_agent(T, &a);
        app.toggle_channel_agent(S, &b);
        app.toggle_channel_agent(S, &a);
        app.toggle_channel_agent(S, &b);
        assert_eq!(app.channel_agents(T).len(), 1);
        assert_eq!(
            app.channel_agents(S)
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            std::slice::from_ref(&a)
        );
        let back = db.open();
        assert_eq!(back.channels.telegram.agents, std::slice::from_ref(&a));
        assert_eq!(back.channels.slack.agents, [a]);
    }
}

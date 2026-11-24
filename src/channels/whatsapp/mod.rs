//! WhatsApp as a channel: Oido linked as a device of a WhatsApp number by scanning a QR code on
//! the Channels screen, the way WhatsApp Web is linked. It speaks WhatsApp Web's protocol through
//! `whatsapp-rust`, which is unofficial: WhatsApp may ban a number used this way, so the number is
//! one of its own, not your personal one. The client runs on Tokio, on a thread of its own, and
//! keeps its session (the keys its messages are encrypted with) in a file of its own, encrypted
//! with a key sealed in the database like a bot token.
//!
//! Only private chats are heard, and the number's own "Message yourself" chat: what the owner
//! writes there from the phone is heard like anyone's, while what this device sent is not. A stranger is not answered: only someone who sends the invite
//! on show (`pair <token>`, written for them by the link its QR code holds) is paired. Buttons go
//! as a poll under the message, and a vote presses one.

pub mod format;

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use diesel::connection::SimpleConnection;
use sha2::{Digest, Sha256};
use whatsapp_rust::prelude::{
    Bot, Client, Event, EventHandler, InboundMessage, Jid, MessageBuilderExt, MessageExt, wa,
};
use whatsapp_rust::store::{SqliteStore, SqliteStoreConfig};
use whatsapp_rust::wacore::poll::compute_option_hash;
use whatsapp_rust::{PollVoteCiphertext, async_channel};
use zeroize::Zeroizing;

use super::{Allowlist, Button, Creds, Health, Heard, Kind, Link, Out, Person, Picture, Place};
use crate::app::App;

/// How often "typing…" is said again while a chat is busy (WhatsApp shows it for about 25 s).
const TYPING_EVERY: Duration = Duration::from_secs(10);
/// The biggest picture or document fetched from someone, in bytes.
const MAX_INCOMING: u64 = 20 * 1024 * 1024;
/// The biggest file an agent sends, in bytes: WhatsApp's limit for a document.
pub const MAX_FILE: u64 = 100 * 1024 * 1024;
/// The biggest picture or video sent to show in the chat; a bigger one goes as a document.
const MAX_MEDIA: u64 = 16 * 1024 * 1024;
/// The most options a WhatsApp poll holds.
const MAX_OPTIONS: usize = 12;
/// How many of the messages this device sent are remembered, so their echoes are not heard.
const SENT_KEPT: usize = 256;
/// What a poll standing in for buttons asks.
const PICK: &str = "Choose one";

/// What the session file is encrypted with: 32 random bytes, in hex.
#[derive(Clone)]
pub struct Key(Zeroizing<String>);

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Key(…)")
    }
}

impl Key {
    /// A new key; `None` when there is no randomness to make one with.
    fn new() -> Option<Key> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        getrandom::fill(bytes.as_mut()).ok()?;
        Some(Key(Zeroizing::new(
            bytes.iter().map(|b| format!("{b:02x}")).collect(),
        )))
    }

    /// Reads a key as [`Key::secret`] writes it.
    pub fn parse(secret: &str) -> Option<Key> {
        (secret.len() == 64 && secret.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| Key(Zeroizing::new(secret.to_ascii_lowercase())))
    }

    /// The key as it is saved (sealed).
    pub fn secret(&self) -> String {
        self.0.to_string()
    }

    /// Names its session file without giving the key away.
    fn id(&self) -> String {
        let hash = Sha256::digest(self.0.as_bytes());
        hash[..8].iter().map(|b| format!("{b:02x}")).collect()
    }

    /// SQLCipher's statement keying a connection with it.
    fn pragma(&self) -> Zeroizing<String> {
        Zeroizing::new(format!("PRAGMA key = \"x'{}'\";", self.0.as_str()))
    }
}

const PREFIX: &str = "whatsapp-";

/// The session file kept with `key` in `folder`: the folder of the database the app has open
/// ([`crate::store::Store::folder`]), never the default one, so a test or a copy on another
/// database never touches the real session.
fn session_file(folder: &Path, key: &Key) -> PathBuf {
    folder.join(format!("{PREFIX}{}.db", key.id()))
}

/// Removes the session file at `path`, with SQLite's files beside it.
fn remove_session(path: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
}

/// Forgets the session kept with `key`. The phone still lists the device until it is removed
/// there (WhatsApp → Linked devices).
pub fn forget(folder: &Path, key: &Key) {
    remove_session(&session_file(folder, key));
}

/// Removes every session file but `keep`'s: those of links never finished, or not unlinked.
/// Run before any link starts.
pub fn sweep(folder: &Path, keep: Option<&Key>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let kept = keep.map(|k| format!("{PREFIX}{}.db", k.id()));
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(base) = name.strip_prefix(PREFIX).and_then(|rest| {
            let db = rest.find(".db")?;
            Some(format!("{PREFIX}{}", &rest[..db + 3]))
        }) else {
            continue;
        };
        if kept.as_deref() != Some(base.as_str()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The address whose QR code takes up invite `token` on the number `bot` (written `+<digits>`):
/// it opens a chat with the number, the invite written in, ready to send.
pub fn invite_link(bot: &str, token: &str) -> String {
    let digits: String = bot.chars().filter(char::is_ascii_digit).collect();
    format!("https://wa.me/{digits}?text=pair%20{token}")
}

impl App {
    /// Starts linking Oido as a device of a WhatsApp number: the Channels screen shows the QR code
    /// to scan, and the link is saved once it is scanned.
    pub fn link_whatsapp(&mut self) {
        let c = &mut self.channels.whatsapp;
        if c.connected() {
            return;
        }
        match Key::new() {
            Some(key) => c.start(Creds::WhatsApp(key)),
            None => {
                c.status =
                    super::Status::Failed("There is no randomness to make a key with.".into());
            }
        }
    }

    /// Stops a link not yet scanned. Its session is dropped.
    pub fn cancel_whatsapp_link(&mut self) {
        let c = &mut self.channels.whatsapp;
        if !c.connected() {
            c.stop();
        }
    }
}

/// Starts the thread that runs the client: it links by QR code or signs in with the session
/// kept with `key`, hands what it hears to [`App::channel_heard`] and sends what the app hands
/// over, tagged with `generation` so a link since replaced is ignored. It ends when the link is
/// dropped.
pub fn start(
    app: rune::AppHandle<App>,
    key: Key,
    folder: Option<PathBuf>,
    allowlist: Allowlist,
    generation: u64,
) -> Link {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let Some(path) = folder.map(|f| session_file(&f, &key)) else {
            report(
                &app,
                generation,
                Health::Stopped("Oido has no folder to keep the session in".into()),
            );
            return;
        };
        run(&app, &key, &path, &allowlist, rx, generation);
    });
    Link::new(Arc::new(AtomicBool::new(false)), tx)
}

fn report(app: &rune::AppHandle<App>, generation: u64, health: Health) {
    app.update(move |s: &mut App, _| s.channel_health(Kind::WhatsApp, generation, health));
}

fn run(
    app: &rune::AppHandle<App>,
    key: &Key,
    path: &Path,
    allowlist: &Allowlist,
    rx: Receiver<Out>,
    generation: u64,
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build();
    let runtime = match runtime {
        Ok(r) => r,
        Err(e) => {
            report(
                app,
                generation,
                Health::Stopped(format!("it could not start: {e}")),
            );
            return;
        }
    };
    // What the app hands over, into the client's world; it closes when the link is dropped.
    let (outbox, inbox) = async_channel::unbounded();
    thread::spawn(move || {
        for out in rx {
            if outbox.send_blocking(out).is_err() {
                break;
            }
        }
    });
    runtime.block_on(session(app, key, path, allowlist, inbox, generation));
}

/// Hands every event the client raises to the session, in order.
struct Forward(async_channel::Sender<Arc<Event>>);

impl EventHandler for Forward {
    fn handle_event(&self, event: Arc<Event>) {
        let _ = self.0.try_send(event);
    }
}

/// A poll sent for buttons: what decrypts its votes, and what each option presses.
struct Poll {
    secret: Vec<u8>,
    /// Who made it, as its votes name it: this device's number or LID.
    creator: Jid,
    options: Vec<([u8; 32], String)>,
}

/// The ids of the messages this device sent lately. WhatsApp echoes them back as the owner's,
/// and in the owner's own chat they would otherwise be heard as written to the agent.
#[derive(Default)]
struct Sent {
    ids: HashSet<String>,
    order: VecDeque<String>,
}

impl Sent {
    fn add(&mut self, id: String) {
        if self.ids.insert(id.clone()) {
            self.order.push_back(id);
        }
        while self.order.len() > SENT_KEPT {
            if let Some(old) = self.order.pop_front() {
                self.ids.remove(&old);
            }
        }
    }

    fn has(&self, id: &str) -> bool {
        self.ids.contains(id)
    }
}

/// How a session ends, for its file.
#[derive(PartialEq)]
enum End {
    /// It can sign in again: keep the file.
    Kept,
    /// Never linked, signed out, or unlinked on the phone: the file goes.
    Gone,
}

async fn session(
    app: &rune::AppHandle<App>,
    key: &Key,
    path: &Path,
    allowlist: &Allowlist,
    inbox: async_channel::Receiver<Out>,
    generation: u64,
) {
    let mut linked = path.exists();
    log(&format!(
        "starting with {} session",
        if linked { "its saved" } else { "a new" }
    ));
    let pragma = key.pragma();
    let config = SqliteStoreConfig::default().with_connection_init(move |conn| {
        conn.batch_execute(&pragma)?;
        // A wrong key fails here rather than later.
        conn.batch_execute("SELECT count(*) FROM sqlite_master;")?;
        Ok(())
    });
    let store = match SqliteStore::with_config(&path.to_string_lossy(), config).await {
        Ok(s) => s,
        Err(e) => {
            report(
                app,
                generation,
                Health::Stopped(format!("its session could not be opened: {e}")),
            );
            return;
        }
    };
    let (events_tx, events) = async_channel::unbounded();
    let bot = Bot::builder()
        .with_backend(store)
        .with_event_handler(Forward(events_tx))
        // The number's past chats are not brought in: only what is written to it from now on.
        .skip_history_sync()
        .build()
        .await;
    let bot = match bot {
        Ok(b) => b,
        Err(e) => {
            report(
                app,
                generation,
                Health::Stopped(format!("it could not start: {e}")),
            );
            return;
        }
    };
    let handle = bot.spawn();
    let client = handle.client();
    let mut polls: HashMap<String, Poll> = HashMap::new();
    let mut sent = Sent::default();
    let mut typing = tokio::time::interval(TYPING_EVERY);
    let mut signed_in = false;
    let end = loop {
        tokio::select! {
            event = events.recv() => {
                let Ok(event) = event else { break End::Kept };
                match &*event {
                    Event::PairingQrCode(qr) => {
                        report(app, generation, Health::Linking(qr.code.clone()));
                    }
                    Event::PairSuccess(_) => {
                        log("linked by its QR code");
                        linked = true;
                    }
                    Event::PairError(e) => {
                        report(app, generation, Health::Stopped(format!("linking failed: {}", e.error)));
                    }
                    Event::PairingQrCodesExhausted(_) => {
                        report(app, generation, Health::Stopped("the QR code was not scanned in time".into()));
                        break End::Gone;
                    }
                    Event::Connected(_) => {
                        log("connected");
                        signed_in = true;
                        let name = client.pn().map_or_else(
                            || client.push_name(),
                            |pn| format!("+{}", pn.user),
                        );
                        report(app, generation, Health::SignedIn(name));
                    }
                    Event::Disconnected(d) => {
                        log(&format!("disconnected: {:?}", d.reason));
                        if signed_in {
                            report(app, generation, Health::Retrying("the connection dropped".into()));
                        }
                    }
                    Event::LoggedOut(out) => {
                        log(&format!("logged out (on connect: {}): {:?}", out.on_connect, out.reason));
                        report(app, generation, Health::Stopped("it was unlinked on the phone".into()));
                        break End::Gone;
                    }
                    // Another client signed in with this session; it is not fought over.
                    Event::StreamReplaced(_) => {
                        log("another client took this session over");
                        report(app, generation, Health::Stopped("another copy of Oido took this link over".into()));
                        break End::Kept;
                    }
                    Event::ConnectFailure(f) => {
                        log(&format!("could not connect: {:?} {:?}", f.reason, f.message));
                    }
                    Event::StreamError(e) => log(&format!("stream error {}", e.code)),
                    Event::TemporaryBan(b) => {
                        log(&format!("temporarily banned: {b:?}"));
                        report(app, generation, Health::Stopped("WhatsApp banned this number for a while".into()));
                        break End::Kept;
                    }
                    Event::ClientOutdated(_) => {
                        report(app, generation, Health::Stopped("WhatsApp turned this version of Oido away".into()));
                        break End::Kept;
                    }
                    Event::Messages(batch) => {
                        let mut heard = Vec::new();
                        for m in batch.messages.iter() {
                            if let Some(h) = hear(&client, m, allowlist, &polls, &sent).await {
                                heard.push(h);
                            }
                        }
                        if !heard.is_empty() {
                            app.update(move |s: &mut App, _| {
                                s.channel_heard(Kind::WhatsApp, generation, heard);
                            });
                        }
                    }
                    _ => {}
                }
            }
            out = inbox.recv() => match out {
                Ok(Out::Unlink) => {
                    client.logout().await;
                    break End::Gone;
                }
                Ok(out) => {
                    let saying = matches!(out, Out::Say { .. });
                    if let Err(why) = send(&client, out, &mut polls, &mut sent).await
                        && saying
                    {
                        app.update(move |s: &mut App, _| {
                            s.channel_send_failed(Kind::WhatsApp, generation, why);
                        });
                    }
                }
                // The link was dropped.
                Err(_) => break End::Kept,
            },
            _ = typing.tick(), if signed_in => {
                app.update(move |s: &mut App, _| s.channel_typing(Kind::WhatsApp, generation));
            }
        }
    };
    handle.shutdown().await;
    if end == End::Gone || !linked {
        log(if linked {
            "session removed: it was unlinked"
        } else {
            "session removed: it was never linked"
        });
        remove_session(path);
    }
}

/// Notes how the link goes, for whoever runs Oido from a terminal.
fn log(what: &str) {
    eprintln!("oido: WhatsApp: {what}");
}

/// Who wrote in a private chat, named the same however WhatsApp addressed them: by their LID
/// (WhatsApp's own id for them) where it is known, else by their number. `None` for anything but
/// a person (a group, a broadcast, a channel).
fn writer(chat: &Jid, alt: Option<&Jid>) -> Option<Jid> {
    let chat = chat.to_non_ad();
    if chat.is_lid() {
        return Some(chat);
    }
    if let Some(alt) = alt.filter(|a| a.is_lid()) {
        return Some(alt.to_non_ad());
    }
    chat.is_pn().then_some(chat)
}

/// Whether a message the owner sent is one to hear: written in their own chat, and not by this
/// device. What they write to others from the phone is theirs, not the agent's.
fn owner_wrote_to_self(chat: &Jid, pn: Option<&Jid>, lid: Option<&Jid>, sent: bool) -> bool {
    !sent
        && match (pn, lid) {
            (Some(pn), lid) => chat.matches_user_or_lid(pn, lid),
            (None, Some(lid)) => chat.matches_user_or_lid(lid, None),
            (None, None) => false,
        }
}

/// What the core hears in a message: a vote on one of the bot's polls, a command, or words and
/// a picture from a private chat or the owner's own chat. Anything else (a group, what this
/// device sent, what the owner wrote to others, a reaction, a receipt) is nothing. A picture is
/// fetched only from someone paired.
async fn hear(
    client: &Arc<Client>,
    m: &InboundMessage,
    allowlist: &Allowlist,
    polls: &HashMap<String, Poll>,
    sent: &Sent,
) -> Option<Heard> {
    let source = &m.info.source;
    if source.is_group {
        return None;
    }
    if source.is_from_me
        && !owner_wrote_to_self(
            &source.chat,
            client.pn().as_ref(),
            client.lid().as_ref(),
            sent.has(&m.info.id),
        )
    {
        return None;
    }
    let who = writer(&source.chat, source.sender_alt.as_ref())?;
    let id = who.to_string();
    let push_name = m.info.push_name.trim();
    let name = if push_name.is_empty() {
        who.user.to_string()
    } else {
        push_name.to_owned()
    };
    let from = Person {
        id: id.clone(),
        first_name: name.split_whitespace().next().unwrap_or(&name).to_owned(),
        name,
        handle: None,
        // Past the check above, a message of the owner's is one written in their own chat.
        owner: source.is_from_me,
    };
    let place = Place::private(&id);
    let base = m.message.get_base_message();

    if let Some(update) = base.poll_update_message.as_option() {
        let poll_id = update.poll_creation_message_key.as_option()?.id.clone()?;
        let poll = polls.get(&poll_id)?;
        let vote = update.vote.as_option()?;
        let ciphertext = PollVoteCiphertext {
            enc_payload: vote.enc_payload.as_deref()?,
            enc_iv: vote.enc_iv.as_deref()?,
        };
        let picked = client
            .polls()
            .decrypt_vote(
                ciphertext,
                &poll.secret,
                &poll_id,
                &poll.creator,
                &source.sender,
            )
            .await
            .ok()?;
        // No option picked: a vote taken back.
        let first = picked.first()?;
        let data = poll
            .options
            .iter()
            .find(|(hash, _)| hash.as_slice() == first.as_slice())?
            .1
            .clone();
        return Some(Heard::Press {
            from,
            id,
            data,
            message: Some((place, poll_id)),
        });
    }

    let at = m.info.timestamp.timestamp();
    let message_id = Some(m.info.id.to_string());
    let image = base.image_message.as_option();
    let document = base.document_message.as_option();
    let text = m
        .message
        .text_content()
        .map(str::to_owned)
        .or_else(|| image.and_then(|i| i.caption.clone()))
        .or_else(|| document.and_then(|d| d.caption.clone()))
        .unwrap_or_default();
    if image.is_none()
        && document.is_none()
        && let Some(command) = text.trim().strip_prefix('/')
    {
        let (name, arg) = command
            .split_once(char::is_whitespace)
            .unwrap_or((command, ""));
        return Some(Heard::Command {
            from,
            place,
            id: message_id,
            at,
            name: name.to_owned(),
            arg: arg.trim().to_owned(),
        });
    }
    // A stranger's file is never fetched; their words may still be an invite.
    let paired = allowlist.read().is_ok_and(|ids| ids.contains(&id));
    let picture = match (image, document) {
        _ if !paired => None,
        (Some(img), _) => Some(
            fetch(
                client,
                img,
                img.file_length,
                format!("whatsapp-photo-{}", m.info.id),
            )
            .await,
        ),
        (None, Some(doc)) => {
            let name = doc
                .file_name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| format!("whatsapp-file-{}", m.info.id));
            Some(fetch(client, doc, doc.file_length, name).await)
        }
        (None, None) => None,
    };
    // Other things a person sends (a voice note, a sticker, a video) are heard, to be turned away
    // in words; what WhatsApp sends on its own (a reaction, an edit, keys) is not.
    let sent_something = picture.is_some()
        || !text.trim().is_empty()
        || base.audio_message.as_option().is_some()
        || base.video_message.as_option().is_some()
        || base.document_message.as_option().is_some()
        || base.sticker_message.as_option().is_some();
    sent_something.then_some(Heard::Message {
        from,
        place,
        id: message_id,
        at,
        text,
        picture,
    })
}

/// Fetches a picture or file someone sent, named `name`, unless it is too big.
async fn fetch(
    client: &Arc<Client>,
    media: &dyn whatsapp_rust::download::Downloadable,
    len: Option<u64>,
    name: String,
) -> Picture {
    let bytes = if len.unwrap_or_default() > MAX_INCOMING {
        Err(format!(
            "it is bigger than {} MB",
            MAX_INCOMING / (1024 * 1024)
        ))
    } else {
        client.download(media).await.map_err(|e| e.to_string())
    };
    Picture { name, bytes }
}

/// Sends one thing. Says why if it did not go.
async fn send(
    client: &Arc<Client>,
    out: Out,
    polls: &mut HashMap<String, Poll>,
    sent: &mut Sent,
) -> Result<(), String> {
    let jid = |place: &Place| -> Result<Jid, String> {
        place
            .chat
            .parse::<Jid>()
            .map_err(|e| format!("{} is not a WhatsApp chat: {e}", place.chat))
    };
    match out {
        Out::Say { to, text, buttons } => {
            let to = jid(&to)?;
            for part in format::render(&text) {
                let done = client
                    .send_message(to.clone(), wa::Message::text(part))
                    .await
                    .map_err(|e| e.to_string())?;
                sent.add(done.message_id);
            }
            if !buttons.is_empty() {
                send_poll(client, &to, &buttons, polls, sent).await?;
            }
            Ok(())
        }
        Out::Typing { to } => client
            .chatstate()
            .send_composing(&jid(&to)?)
            .await
            .map_err(|e| e.to_string()),
        // A press's note goes to who pressed, as a message: polls have no note of their own.
        Out::Answered {
            callback,
            note: Some(note),
        } => client
            .send_message(
                jid(&Place::private(&callback))?,
                wa::Message::text(format!("_{note}_")),
            )
            .await
            .map(|done| sent.add(done.message_id))
            .map_err(|e| e.to_string()),
        // A poll cannot be taken back; each button still works once (the core sees to that).
        Out::Answered { note: None, .. } | Out::Unbutton { .. } | Out::Working { .. } => Ok(()),
        Out::Unlink => Ok(()),
        Out::File {
            to,
            path,
            name,
            caption,
            reply,
        } => {
            let sent = match jid(&to) {
                Ok(to) => send_file(client, to, &path, name, caption, sent).await,
                Err(why) => Err(why),
            };
            reply.answer(sent);
            // How it went is the job's to say, not a failed message's.
            Ok(())
        }
    }
}

/// How a file shows in the chat.
#[derive(Debug, PartialEq)]
enum Shown {
    Image(&'static str),
    Video,
    Document,
}

/// How file `name`, `len` bytes long, is sent: a picture or an MP4 small enough shows in the
/// chat, anything else comes as a document to download.
fn shown(name: &str, len: u64) -> Shown {
    let lower = name.to_lowercase();
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    if len > MAX_MEDIA {
        return Shown::Document;
    }
    match ext {
        "png" => Shown::Image("image/png"),
        "jpg" | "jpeg" => Shown::Image("image/jpeg"),
        "webp" => Shown::Image("image/webp"),
        "mp4" => Shown::Video,
        _ => Shown::Document,
    }
}

/// Uploads the file at `path` and sends it to `to`.
async fn send_file(
    client: &Arc<Client>,
    to: Jid,
    path: &Path,
    name: String,
    caption: Option<String>,
    sent: &mut Sent,
) -> Result<(), String> {
    use whatsapp_rust::download::MediaType;
    use whatsapp_rust::media::{self, DocumentOptions, ImageOptions, VideoOptions};

    let path = path.to_path_buf();
    let bytes = tokio::task::spawn_blocking(move || std::fs::read(path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("it could not be read: {e}"))?;
    let how = shown(&name, bytes.len() as u64);
    let kind = match how {
        Shown::Image(_) => MediaType::Image,
        Shown::Video => MediaType::Video,
        Shown::Document => MediaType::Document,
    };
    let up = client
        .upload(bytes, kind, whatsapp_rust::UploadOptions::default())
        .await
        .map_err(|e| e.to_string())?;
    let message = match how {
        Shown::Image(mime) => media::image_message(
            up,
            ImageOptions {
                caption,
                mimetype: Some(mime.into()),
                ..Default::default()
            },
        ),
        Shown::Video => media::video_message(
            up,
            VideoOptions {
                caption,
                ..Default::default()
            },
        ),
        Shown::Document => media::document_message(
            up,
            DocumentOptions {
                title: Some(name.clone()),
                file_name: Some(name),
                caption,
                ..Default::default()
            },
        ),
    };
    let done = client
        .send_message(to, message)
        .await
        .map_err(|e| e.to_string())?;
    sent.add(done.message_id);
    Ok(())
}

/// Sends `buttons` as a poll to `to`, one option each, and keeps what decrypts its votes.
async fn send_poll(
    client: &Arc<Client>,
    to: &Jid,
    buttons: &[Button],
    polls: &mut HashMap<String, Poll>,
    sent: &mut Sent,
) -> Result<(), String> {
    let mut names: Vec<String> = Vec::new();
    for b in buttons.iter().take(MAX_OPTIONS) {
        // A poll's options must differ.
        let mut name = b.label.clone();
        let mut n = 2;
        while names.contains(&name) {
            name = format!("{} ({n})", b.label);
            n += 1;
        }
        names.push(name);
    }
    let (poll, secret) = client
        .polls()
        .create(to.clone(), PICK, &names, 1)
        .await
        .map_err(|e| e.to_string())?;
    sent.add(poll.message_id.clone());
    let creator = if poll.to.is_lid() {
        client.lid()
    } else {
        client.pn()
    };
    let Some(creator) = creator else {
        return Ok(());
    };
    let options = names
        .iter()
        .zip(buttons)
        .map(|(name, b)| (compute_option_hash(name), b.data.clone()))
        .collect();
    polls.insert(
        poll.message_id,
        Poll {
            secret,
            creator,
            options,
        },
    );
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_key_reads_back_as_written_and_never_shows() {
        let key = Key::new().unwrap();
        let again = Key::parse(&key.secret()).unwrap();
        assert_eq!(again.id(), key.id());
        assert_eq!(format!("{key:?}"), "Key(…)");
        assert!(!key.id().is_empty() && !key.secret().contains(&key.id()));
        assert!(Key::parse("short").is_none());
        assert!(Key::parse(&"z".repeat(64)).is_none());
    }

    #[test]
    fn an_invite_link_opens_the_number_with_the_invite_written() {
        assert_eq!(
            invite_link("+357 99 123456", "abc123"),
            "https://wa.me/35799123456?text=pair%20abc123"
        );
    }

    #[test]
    fn a_person_is_named_by_their_lid_where_known() {
        let pn: Jid = "35799123456@s.whatsapp.net".parse().unwrap();
        let lid: Jid = "123456789@lid".parse().unwrap();
        let group: Jid = "1203630@g.us".parse().unwrap();
        assert_eq!(writer(&lid, Some(&pn)), Some(lid.clone()));
        assert_eq!(writer(&pn, Some(&lid)), Some(lid));
        assert_eq!(writer(&pn, None), Some(pn));
        assert_eq!(writer(&group, None), None);
    }

    #[test]
    fn the_owner_is_heard_only_in_their_own_chat_and_never_this_devices_echo() {
        let pn: Jid = "35799123456@s.whatsapp.net".parse().unwrap();
        let lid: Jid = "123456789@lid".parse().unwrap();
        let friend: Jid = "35799000000@s.whatsapp.net".parse().unwrap();
        assert!(owner_wrote_to_self(&pn, Some(&pn), Some(&lid), false));
        assert!(owner_wrote_to_self(&lid, Some(&pn), Some(&lid), false));
        assert!(
            !owner_wrote_to_self(&pn, Some(&pn), Some(&lid), true),
            "an echo"
        );
        assert!(!owner_wrote_to_self(&friend, Some(&pn), Some(&lid), false));
        assert!(
            !owner_wrote_to_self(&pn, None, None, false),
            "not signed in"
        );

        let mut sent = Sent::default();
        for n in 0..=SENT_KEPT {
            sent.add(n.to_string());
        }
        assert!(!sent.has("0") && sent.has(&SENT_KEPT.to_string()));
    }

    #[test]
    fn pictures_and_mp4s_show_in_the_chat_and_the_rest_come_as_documents() {
        assert_eq!(shown("launch.MP4", 1000), Shown::Video);
        assert_eq!(shown("a.png", 1000), Shown::Image("image/png"));
        assert_eq!(shown("a.jpeg", 1000), Shown::Image("image/jpeg"));
        assert_eq!(shown("a.mov", 1000), Shown::Document);
        assert_eq!(shown("report", 1000), Shown::Document);
        assert_eq!(shown("big.mp4", MAX_MEDIA + 1), Shown::Document);
    }
}

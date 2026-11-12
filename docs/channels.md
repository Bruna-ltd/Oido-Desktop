# Channels

Paired people talk to chosen agents from a messaging app: Telegram, Slack and WhatsApp.

**Code.** `src/channels/mod.rs` is the part no channel owns: pairing, which agent each person talks
to, their chats, commands, and what goes back when a turn is over. A channel turns what it hears
into `Heard` (a message, a command, a button press) and sends each `Out` (say, typing, unbutton,
answered) its own way, from its own thread; replies go to a `Place` (a chat, and a thread in it).
`press.rs` is the buttons (questions, drafts), `send.rs` the `send_file` tool. Each channel lives
in its own folder: `telegram/` (`api.rs` the Bot API, `format.rs` Markdown to Telegram HTML,
`mod.rs` connecting, its threads, and updates turned into `Heard`), `slack/` (the same, for
Slack's Web API, mrkdwn and Socket Mode) and `whatsapp/` (a linked device, below). `mrkdwn.rs`
is the Markdown shape Slack and WhatsApp share, each with its own escaping and links. Storage is
`src/store/channels.rs` (rows name their channel), the screen `src/ui/channels.rs`.

## Telegram

**Connecting.** The Channels screen (in the rail) takes a bot token from @BotFather. It is checked with `getMe`
first: a token that does not work is not saved. A token that works is sealed in the database like
provider keys (`channel_bots.token_enc`, bound to `channel:telegram`). The bot's `/` menu is set
from `COMMANDS`. Disconnecting forgets the token; paired people stay paired for the next bot.

**Listening.** The bot runs inside the app: one thread long-polls `getUpdates` (50 s), one sends
messages in order, one refreshes "typing…" every 4 s while a bound chat is busy. With no public
address there is no webhook, so the bot is offline whenever Oido is closed. Asking for updates past
one confirms it to Telegram, so nothing is handled twice and no offset is stored. Each link has a
generation: what an old link hears or says after a disconnect or a new token is ignored. A revoked
token stops the poller and says so on the Channels screen; network failures retry with backoff (up to 60 s).

**Who is answered.** Private chats only; groups and bots are ignored. A stranger gets a six-digit
code (good for 10 minutes, at most 10 waiting) and a desktop notification asks you to approve it on
the Channels screen. Approving stores their Telegram user id (`channel_users`); everyone else is ignored.
Unpairing is one click. A paired person can use the agent's tools on this computer, so the
allowlist is the trust boundary.

**Which agent.** Each channel has its own agents, chosen on the Channels screen (`channel_agents`;
agents of their own only, a team member is reached through its lead). An agent cannot add itself
to a channel, and one an agent creates is on none. `/agent` offers a channel's agents as buttons,
`/agent <name>` picks by name or prefix; with one chosen it is used without asking.

**Chats.** Each person carries on one chat with each agent (`channel_chats`): an ordinary chat in
the agent's list, titled "Telegram · <name>" and marked in its tab. `/new` binds a fresh one; the
old one stays, unmarked. Deleting the chat or the agent, or unpairing, ends the binding.

**A turn.** A message goes to the bound chat. If the agent asked a question, the message answers
it. If the agent is still working, the message waits and goes when the reply ends; `/stop` stops
the reply and drops anything waiting. When nothing more runs in the chat, everything the agent
wrote since the message goes back (`App::channel_turn_over`, called from `settle`), with its
question and options, "Stopped." or the failure. A reply to a message typed in the app goes too. No
desktop notification is raised for these turns; pairing requests still notify.

**Buttons.** A question with options to pick one of gets a button per option; a question that
takes several lists them, and "1, 3" written back picks those (so does "2" for one). Writing
anything else answers in those words. What the agent drafted in the turn follows its reply, each
with its buttons: an agent (Create / Reject, Delete / Keep it), a schedule (Approve / Reject), and a
workflow once it is learned (Save / Discard; learning runs after the turn, so it comes when ready).
Approving does what the card's button does in the app; if it can no longer be done, the reason is
sent and the draft is offered again. Editing a draft stays in the app. MCP server drafts are not
offered: Telegram says one waits in the app, as a local server runs a program on the computer.
Each button belongs to the person it was sent to and works once; pressing one takes its message's
buttons away (`src/channels/press.rs`). Buttons live while the app runs: after a restart a press is
turned away, and a question can still be answered by writing.

**Pictures.** A photo, or a picture sent as a file, goes to the agent with its caption, shrunk
like one attached in the app. The poller fetches it before the app hears the message, so messages
keep their order, and only from people paired (it reads a shared copy of the allowlist), up to
10 MB. One that cannot be fetched or read is not sent, and the sender is told why. A picture sent
while a question waits goes after the answer. Other kinds (voice, stickers, other files) are turned
away for now.

**Sending files.** In a chat carried on from Telegram, the agent is also offered `send_file`
(`src/channels/send.rs`): a file from its working folder (the home folder if it has none, as for
commands; nothing outside it) goes to the person, a PNG, JPEG or WebP picture as a photo (as a file
if Telegram will not take it as one), anything else as a file, up to 50 MB, with an optional
caption. It runs as a job, like a command: the chat is busy until the upload ends, and the agent
hears whether it went. Elsewhere the tool is not offered, and a call to it says why.

**Formatting.** The core writes Markdown; Telegram's sender turns it into Telegram HTML (only `<`, `>`, `&` need escaping, unlike MarkdownV2):
bold headings, bullets, `<pre>` code and tables, links. Each message holds at most 4096 UTF-16
units, so a reply is cut between blocks, and a block too long on its own between lines. If Telegram
cannot parse a part, the same words go unformatted.

**Missed messages.** Telegram keeps updates for 24 hours. A message older than 10 minutes when it
is heard was sent while Oido was closed: it is not run, and the sender is asked to send it again.

## Slack

**Connecting.** The Channels screen copies a manifest to make the Slack app from (api.slack.com/apps
→ Create New App → From a manifest): a bot that hears direct messages and @mentions through Socket
Mode, the `/oido` command, and buttons. Once the app is installed, its bot token (`xoxb-`) and an
app-level token with `connections:write` (`xapp-`) are pasted back. Both are checked first
(`auth.test`, `apps.connections.open`), then sealed together as one secret.

**Listening.** Socket Mode is a WebSocket the app opens (`tungstenite`, blocking, on its own
thread), so Slack needs no public address either. Each envelope is acknowledged before any work on
it; a `disconnect` or a broken socket opens a new one, with backoff. Messages go out from one
thread, as Block Kit sections of mrkdwn (at most 2900 characters each, 45 to a message), buttons as
Block Kit buttons under the last. Slack has no "typing…" for bots: a message being worked on wears
a 👀 reaction until the agent is done with it (it stays while a message waits to go next).

**Where it answers.** Direct messages, and @mentions in channels the bot was invited to, answered
in the mention's thread. From a mention the agent gets only what the person wrote, never the rest
of the thread, so others in the channel cannot steer it. What the agent writes goes to the thread;
its questions and drafts go to the person privately, and so does a stranger's pairing code.
Commands are `/oido agent|new|stop|help` (answered privately wherever they are typed), or those
words alone. A press takes its message's buttons off at once; Slack has no note on a press, so a
note goes to the person as a message.

**Pictures and files.** A picture shared in a direct message (or with a mention) goes to the agent
with the message's words: the first image of what was shared, fetched with the bot token from
Slack's file host only (`files.slack.com`), only for people paired, up to 10 MB. In a direct
message the agent is offered `send_file`: Slack's upload in three steps (an address for the file,
the bytes, then the file shared in the person's private chat), up to 50 MB. Never into a channel's
thread. Both need the `files:read` and `files:write` scopes in the manifest; an app made from an
older manifest says `missing_scope` until it is updated and reinstalled.

## WhatsApp

**Unofficial.** Oido links to a WhatsApp number as a device, the way WhatsApp Web does, through
`whatsapp-rust` (WhatsApp Web's protocol, reverse-engineered). WhatsApp's official API only
delivers messages to a webhook, so a public address; a linked device needs none, but WhatsApp may
ban a number used this way. The Channels screen says so before linking: link a number of its own,
never your personal one.

**Linking.** "Link…" starts the client with a new key, and the Channels screen shows the QR code
WhatsApp hands out (a new one every few seconds) to scan from the phone (Settings → Linked devices
→ Link a device). Once it signs in, the number is saved like a bot: `channel_bots` holds the key,
sealed, and the name `+<number>`. The client keeps its session (the keys its messages are
encrypted with) in a file of its own beside the database, `whatsapp-<id>.db`, encrypted with that
key (SQLCipher: the SQLite Oido links is SQLCipher, and a database opened without a key, like
`oido.db`, is plain SQLite as before). Cancelling, a code never scanned, disconnecting (which
unlinks the device on the phone too) and unlinking on the phone all remove the file; one left by
a link never finished is removed at the next start. The number's past chats are not brought in.

**Listening.** The client runs on Tokio, on a thread of its own, while the app runs; WhatsApp
keeps what is sent while Oido is closed and hands it over on the next start (older than 10 minutes:
the sender is asked to send it again). The connection comes back on its own after a drop.

**Who is answered.** Private chats only. A stranger is not answered at all: the number may be one
people know, so nobody gets a pairing code. "Invite by QR" shows a code holding
`wa.me/<number>?text=pair <token>`: scanned with a phone's camera, it opens a chat with the number,
the invite written in. Whoever sends it first is paired, at once, and a desktop notification says
so. An invite is 80 random bits, works once, for 10 minutes, one on show at a time; taking it back
or a restart ends it. A person is known by their LID (WhatsApp's own id for them) where WhatsApp
gives it, else by their number.

**A turn, commands, buttons.** As on Telegram: `/agent`, `/new`, `/stop`, `/help`, "typing…" while
the agent works, replies in WhatsApp's markup (`*bold*`, `_italic_`, `~struck~`, `` `code` ``, fenced
code; links as their words with the address after; at most 4000 characters a message). WhatsApp
has no buttons for a number like this, so the buttons go as a poll under the message ("Choose
one"), and a vote presses one. A poll cannot be taken back; each still works once, and a note on a
press comes as a message. Polls live while the app runs.

**Pictures.** A picture from someone paired goes to the agent with its caption, up to 10 MB;
voice notes, stickers and files are turned away in words. `send_file` is not offered on WhatsApp
yet.

## Not yet

- Voice notes (would need speech to text), other files in, Telegram groups.
- `send_file` on WhatsApp (a file would go through the running client).

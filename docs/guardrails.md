# Guardrails

Checks made on this computer, on both sides of the model: what goes to a model, and what an agent
asks to do, are looked at before either happens. Nothing here asks a model; the rules are patterns
run locally, so they are fast, free and private.

Status: phases 1 to 3 built 2026-11-11 (secrets hidden from models, risky actions held or refused;
taint, trust switches, a time limit on go-aheads, go-aheads for work nobody watches; personal
details, hidden instructions, blocked words, limits and topics for channels, a check on replies to
channels).

## Before

- Every message, channel text and tool result went to the provider as it was: a key pasted into a
  chat, or read from a `.env` file by a tool, went with it.
- Every tool call ran as soon as the model asked: `shell`, file writes, MCP tools, web fetches, with
  no go-ahead from anyone.

## The rule

**Chats keep the real values; only models see placeholders.**

- Going out, every model call (a reply, a summary, a team member's call, a workflow's model step)
  passes through `Guard::request`, the only way to make the `Guarded` request that
  `completion::run` and `completion::complete` take. Secrets in it are replaced with placeholders
  like `<<SECRET_1>>`, and the model is told what they are and to use them as written.
- Coming back, what the model wrote is revealed (placeholders turned back into the values) before
  it is saved or acted on, and each tool call is reviewed by `Guard::review` before it runs.
- Placeholders are numbered by where a value is first seen in the chat (the brief first, then the
  messages in order), and worked out again for each call rather than stored. The same value is
  always the same placeholder, so what was sent before does not change and the provider's prompt
  cache keeps working.
- The run records what was actually sent: placeholders, never the values.

## The packs

Each pack is switched on or off on the Guardrails screen (the shield on the rail), for every agent,
chat, schedule, workflow and channel at once. The first four start on; "Hide personal details" and
"Check replies to channels" start off, since they can get in the way.

**Hide secrets from models** (`Pack::Secrets`, `guard/secrets.rs`). Found by shape only, so ordinary
text is left alone:

- provider keys and tokens by their prefix: `sk-…` (OpenAI, Anthropic, OpenRouter), `ghp_…` and
  `github_pat_…`, `glpat-…`, `hf_…`, `gsk_…`, `npm_…`, Slack's `xox?-…` and `xapp-…`, AWS `AKIA…`,
  Google `AIza…`, Stripe `sk_live_…`, a Telegram bot token, a JWT;
- a private key block (`-----BEGIN … PRIVATE KEY-----` to its end);
- the password in a URL (`postgres://user:password@host`);
- a value written as `password = …`, `API_KEY: "…"`, `client_secret=…`, when it looks like one
  (8 or more characters, letters and digits, not code or a variable), so `token = self.token` is
  left alone.

A tool gets the real value back in its arguments (`write_file` writing a `.env`, `shell` using a key).
A placeholder going to a tool that sends it away from this computer (a web fetch, an MCP tool,
`send_file`, TypeSafe) waits for your go-ahead. A placeholder this chat does not have (the model
made one up) is refused.

**Stop risky actions** (`Pack::Actions`, `guard/actions.rs`). A command or file write that can do
lasting harm waits for your go-ahead:

- running as administrator (`sudo`, `doas`, `pkexec`, `su`);
- deleting outside the working folder, a whole home or system folder, or a path held in a variable;
- running code straight from the internet (`curl … | sh`, `bash -c "$(curl …)"`);
- changing system files, or ones that control logins, keys or what starts on its own (`~/.ssh`,
  `~/.bashrc`, `~/.config/autostart`, `/etc/…`), by the shell or a file tool;
- changing a git hook or git's settings; force-pushing; throwing away git work (`reset --hard`,
  `clean -f`, `branch -D`);
- changing permissions across a large folder; stopping the computer; changing services or cron.

Erasing a disk (`mkfs`, `wipefs`, `dd of=/dev/…`), a fork bomb and `rm --no-preserve-root` are refused
outright. The patterns catch the common shapes; a command can always be written another way, so this
is a net, not a wall.

**Ask before acting on outside text** (`Pack::Taint`, `guard/taint.rs`). Text from outside can carry
instructions meant to steer the agent (prompt injection). No pattern can tell them apart from
ordinary text, so instead: while any outside text is in what a model is sent, every call it makes
that acts waits for your go-ahead.

- Outside text is a web page, an MCP tool's result, a message (or an answer to a question) from a
  channel person you have not marked trusted, and, in a folder you have not trusted, a file the agent
  read or a command's output. A team member's answer is outside text when it had any in view.
- A call acts when it runs a command, writes or edits a file, fetches a page, sends a file, runs a
  workflow, hands work to a team member, uses an MCP tool or TypeSafe. Reading a file and asking you
  do not act. Drafting a schedule, an agent or an MCP server does not either: the draft already waits
  for your approval.
- Each result records whether it is outside text, and what (`ToolCall::outside`), as it arrives. For
  each model call, the first outside thing still in what is sent is its taint (`App::tainted`), so it
  lasts as long as the text does: a result trimmed to save room no longer counts, a summary of
  outside text carries it on (`Message::Summary::outside`), and Start fresh leaves it behind.
- A team member starts with its lead's taint, adds its own, and hands it back with its answer.
- Workflow steps are fixed by you, not chosen by a model, so taint does not hold them.

Two switches say what counts as yours:

- **Trust what it reads and runs there**, per agent, under its working folder (on). Turn it off for
  a folder of things you did not write: downloads, a cloned project. Only you can change it; an
  agent changing agents cannot.
- **Trusted**, per paired person on the Channels screen (off). Their messages then count as yours.

**Catch hidden instructions** (`Pack::Injection`, `guard/injection.rs`, on). Characters a person
cannot see (the Unicode tag block, used to spell out words nobody sees; direction overrides;
zero-width spaces) are taken out of everything a model is sent, MCP tools' descriptions included;
the chat keeps them, and the run says how many went. Text that phrases an order to an agent
("ignore all previous instructions", "you are now in developer mode", a fake `<system>` block,
"do not tell the user") makes a result outside text even from a trusted folder, and says so: "a
file it read that tries to give it orders (…)". An MCP tool whose description does it keeps outside
text in view for as long as its server is there. Phrasings can change, so taint is what holds.

**Hide personal details from models** (`Pack::Personal`, off). Email addresses, international
phone numbers (`+357 …`, `0044 …`), card numbers that pass their check digit and IBANs that pass
their checksum are hidden like secrets, as `<<EMAIL_2>>`, `<<PHONE_3>>`, `<<CARD_1>>`,
`<<IBAN_4>>`, but only when they come in outside text: what you type yourself goes as it is.

**Blocked words** (a list on the Guardrails screen, empty to start). Names of clients or
competitors, internal code names, words you do not want used: matched as whole words in any case,
hidden from every model as `<<WORD_n>>` and shown back in your chat, and left as the placeholder in
a reply to a channel person who did not write them.

**Channels.** Limits on what one paired person may send (Guardrails screen, off until set): the
longest message (500, 2,000 or 8,000 characters) and how many an hour (10, 30 or 100). On an
agent's form, "On channels, only answer about" lists its topics: a channel message that names none
of them is turned away, unless it answers a question the agent asked. All of these answer with a
short reply on the spot; no model is asked (`guard/intake.rs`).

**Check replies to channels** (`Pack::Replies`, off, `guard/replies.rs`). A reply to a channel
person that repeats 60 or more characters of the agent's brief and skills word for word is held
back (they are told so; your chat has it). Paths on this computer (`/home/…`, `~/…`) and local
network addresses (`localhost`, `192.168…`, `*.local`) in a reply are left out.

## What you see

- A refused call has a "Refused by a guardrail" card under it; the model is told it did not run and
  not to work around it.
- A held call has a "Needs your go-ahead" card with **Allow** and **Don't allow**. Every call in the
  same reply waits with it, so nothing runs out of order. The agent shows "Waiting for your
  approval", the composer waits too, and a desktop notification says it needs you. Allowed, the
  calls run as they would have; turned down, the model is told "the person said no" and carries on.
- Nobody answers within the wait (Guardrails → Wait for your answer: 5, 10, 30 or 60 minutes; 10 to
  start), and the call does not run: the model is told nobody gave the go-ahead. A held call
  survives a restart, and the wait is counted from when it began.
- In a chat carried on from a channel, the person is told it waits for approval on the computer
  running Oido; only you can give it. A reply sent back to them carries only the secrets they sent
  themselves; any other stays its placeholder.
- A team member working on its own, or a workflow step, waits on a card of its own ("checker needs
  your go-ahead"): in the chat whose agent handed the task over or ran the workflow, or on the
  workflow's page when you or a schedule ran it. A desktop notification says so. Stopping the work
  takes the card away. With no app running to show the card, the call does not run.
- The Runs screen shows a GUARD step for each: "1 secret hidden from the model", "Outside text in
  view", "Runs as administrator: waiting for your go-ahead", "…: you allowed it", "…: nobody
  answered in time", "…: refused".

## Where it lives

| What | Where |
| --- | --- |
| Packs, trips, the guarded request, reviewing a call | `src/guard/mod.rs` |
| Finding and hiding secrets | `src/guard/secrets.rs` |
| Risky commands and paths | `src/guard/actions.rs` |
| Outside text, and which calls act | `src/guard/taint.rs` |
| Reviewing a reply's calls, holding, deciding and timing them out; taint of a model call | `src/guard/gate.rs` |
| Go-aheads for work nobody watches | `src/guard/ask.rs` |
| Hidden characters and orders in text | `src/guard/injection.rs` |
| Limits and topics for channel messages | `src/guard/intake.rs` |
| What a reply to a channel must not carry | `src/guard/replies.rs` |
| Trip cards and the Guardrails screen | `src/ui/guard.rs` |
| Which packs are on; how long a go-ahead is waited for; blocked words; channel limits | settings `guard.packs`, `guard.wait`, `guard.words`, `guard.channel_limits` |
| An agent's channel topics | `agents.channel_topics` (migration 020) |
| A call's trip, and whether its result is outside text | kept with the call in the chat (`ToolCall::trip`, `ToolCall::outside`) |
| Trusted folders and people | `agents.trust_workdir`, `channel_users.trusted` (migration 019) |

## Not covered yet

- Secrets inside pictures (nothing on this computer reads text in images).
- A command written to avoid the patterns, in a chat with no outside text in view: you, or a
  trusted person, asked for it.
- What a workflow step fetches feeding a later step: the steps are yours, so taint does not hold them.
- Personal details in what a workflow step fetches: a step's model call keeps its own hidden values,
  and does not know which text came from outside.
- Local phone numbers (`99 123456`): only international ones are recognised.
- The hourly limit is counted in memory and starts again when Oido does.

## Next

- **Phase 4.** Your own rules (patterns and words), per-agent overrides, and "Always allow this"
  on a trip card.

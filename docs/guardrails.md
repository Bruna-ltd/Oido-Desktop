# Guardrails

Checks made on this computer, on both sides of the model: what goes to a model, and what an agent
asks to do, are looked at before either happens. Nothing here asks a model; the rules are patterns
run locally, so they are fast, free and private.

Status: phases 1 and 2 built 2026-11-11 (secrets hidden from models, risky actions held or refused;
taint, trust switches, a time limit on go-aheads, and go-aheads for work nobody watches).

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
chat, schedule, workflow and channel at once. All three start on.

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
| Trip cards and the Guardrails screen | `src/ui/guard.rs` |
| Which packs are on; how long a go-ahead is waited for | settings `guard.packs`, `guard.wait` |
| A call's trip, and whether its result is outside text | kept with the call in the chat (`ToolCall::trip`, `ToolCall::outside`) |
| Trusted folders and people | `agents.trust_workdir`, `channel_users.trusted` (migration 019) |

## Not covered yet

- Secrets inside pictures (nothing on this computer reads text in images).
- A command written to avoid the patterns, in a chat with no outside text in view: you, or a
  trusted person, asked for it.
- What a workflow step fetches feeding a later step: the steps are yours, so taint does not hold them.

## Next

- **Phase 3.** Personal details (email, phone, card, IBAN) hidden in untrusted text; prompt-injection
  phrases and hidden characters flagged; channel limits (message length, rate), a per-agent topic
  allowlist, a blocklist, and a check that a reply to a channel does not echo the brief.
- **Phase 4.** Your own rules (patterns and words), per-agent overrides, and "Always allow this"
  on a trip card.

# Oido Desktop

A native desktop app for running agents, written in Rust on the [Rune](https://github.com/anomalyco/rune)
GPU UI framework. It is its own app: it does not talk to `oido-core`. It keeps its agents, chats,
providers and settings in a local SQLite database, and talks to model providers directly.

## Install

```
curl -fsSL https://raw.githubusercontent.com/Bruna-ltd/Oido-Desktop/main/scripts/install.sh | bash
```

Downloads the latest release for this machine, checks its SHA-256, installs it at
`~/.local/bin/oido-desktop` and adds Oido to the app launcher (`OIDO_INSTALL_DIR` for another folder,
`OIDO_VERSION=0.2.1` for a particular release). Linux only; it needs OpenSSL 3 (`libcrypto.so.3`).
From then on it updates itself. `oido-desktop --register` adds it to the launcher again, from wherever
it is, without opening the window (0.2.1 and later; older ones add themselves at each launch). The repository must be public for the script and
the updater to see its releases. To remove it, delete the binary, `~/.local/share/applications/oido-desktop.desktop`
and, if you want your agents and chats gone too, `~/.local/share/oido-desktop`.

## Run

```
cargo run                      # opens the window
cargo watch -x run             # restarts on save (cargo install cargo-watch)
OIDO_DB=:memory: cargo run -- --snap out.png --scene providers --size 1280x900   # a headless render (debug builds; --scene NAME shows sample data; without OIDO_DB it opens, and migrates, your own database)
```

Closing the window leaves Oido in the bar's tray (schedules keep firing, connected Telegram and Slack bots keep answering; see `docs/channels.md`); click the icon to bring it
back, and quit from its menu or with Ctrl+Q. Without a tray, closing quits. A reply that finishes while
the window is hidden or unfocused raises a desktop notification; clicking it opens the chat. Launching it again while it
runs shows the running app's window: one app per database. Release builds add Oido to the app launcher
(`~/.local/share/applications/oido-desktop.desktop` and its icon, from `assets/icon/`).

Rune is pre-release, so `Cargo.toml` pins one revision of its `dev` branch. Bump it on purpose.

## Test

```
cargo test                     # unit and end-to-end tests; never touch your data or keyring
cargo test -- --ignored        # also exercises the real system keyring (needs a Secret Service)
cargo clippy --all-targets
```

## Release

`.github/workflows/release.yml` can publish a release for every merge to `main` (not for changes to
only docs, the workflows or `scripts/install.sh`). **It is off:** the tests locked up the runner, so
until that is fixed only the Actions tab's "Run workflow" releases `main` as it is. To turn merges on:
`gh variable set AUTO_RELEASE --body true`. Its version is `Cargo.toml`'s when no release has it yet,
so bump it in a pull request for a minor or major release; otherwise it is the latest release's with
the patch number one higher.

By hand, from a clean `main` with `../rune` in place (see below):

```
scripts/release.sh 0.2.0       # bump Cargo.toml, test, build, tag v0.2.0, push, publish the GitHub release
oido-desktop --version
```

`Cargo.toml` patches Rune to `../rune`: the pinned revision with our scroll speed. A runner has no
such folder, so the workflow fetches that revision and applies `patches/rune-wheel-notch.patch`.
After changing the copy, refresh the patch: `git -C ../rune diff <rev> > patches/rune-wheel-notch.patch`.

`Cargo.toml`'s version is the app's version. A release is a GitHub release tagged `vX.Y.Z` holding
`oido-desktop-<arch>-linux` and its `.sha256`. Installed apps look for one at launch and every six
hours (Settings → Updates; on unless turned off), download it, check its SHA-256, and rename it over
their own binary, so the next launch runs it ("Restart now" does that at once). The repository must
be public for apps to see its releases. Debug builds never update themselves, and a binary in a folder
the user cannot write to (a package manager's) reports why it cannot.

## Where things live

| What | Where |
| --- | --- |
| Database | `~/.local/share/oido-desktop/oido.db` (override with `OIDO_DB`, `:memory:` for a throwaway) |
| Schema and queries | `db/migrations/*.sql` and `db/queries/*.sql`, run by `src/store` |
| Provider API keys | sealed in the database (XChaCha20-Poly1305); the key that opens them is in the OS keyring |
| Runs | each message you send is a run, lasting until the agent stops or waits on you: its setup (the brief, each skill, the context, the tools offered), then every model call (messages sent, reply, thinking, tokens, time to first token) and every tool call (arguments, result, time), drawn on a timeline on the Runs screen. Kept 30 days, then deleted at startup; chats are never touched. Keys are never recorded |
| Usage | every model call is added to daily totals (by model, agent and source) and to its chat's total as it is recorded; kept after runs expire, until cleared in Settings. Cost is estimated when shown, from [models.dev](https://models.dev) prices (fetched at most daily, kept in the database) or a price you set on the Usage screen. See `docs/usage.md` |
| Channels | a Telegram bot's token and a Slack app's tokens sealed like provider keys; the people paired with it and the chat each carries on with each agent. See `docs/channels.md` |
| Guardrails | which packs are on, how long a go-ahead waits, blocked words and channel limits (Guardrails screen, settings); whether an agent's folder (agent form) and a paired person (Channels) count as yours, and an agent's channel topics (agent form). A tool call a rule refused or held keeps its trip in the chat, and a result that is outside text says so. Secrets are hidden from models and kept as they are in chats. See `docs/guardrails.md` |
| Workflows | saved in the database with the agent that learned them (deleted with it); drafts wait in their chat; runs are kept 30 days |
| Model lists | fetched from each provider when it is connected, saved in the database, refreshed when stale |

A provider is connected only after its key has listed models: a key that cannot is not saved. If the
system keyring is unavailable, no key is stored.

Two providers can also be signed in to with a subscription instead of a key, and their sign-in is
sealed and renewed like a key: Anthropic with a Claude Pro/Max plan (paste back the code the browser
shows), and OpenAI Codex with a ChatGPT Plus, Pro or Business plan. Codex signs in as the Codex CLI's
own OAuth client, so the browser comes back to port 1455 of this computer (nothing to paste, but the
port must be free, which a Codex CLI sign-in waiting in a terminal is not), and its calls go to
ChatGPT's Codex backend, which OpenAI does not document, so it may change without notice.

## Layout

- `src/agents.rs`, `src/providers.rs`, `src/models.rs`: the domain (agents and teams, providers,
  models and what they can do).
- `src/store/`: SQLite, migrations and typed queries. `src/vault.rs`: sealing keys.
- `src/listing.rs`: asking a provider for its models. `src/connect.rs`: connecting, refreshing.
  `src/credentials.rs`, `src/chatgpt.rs`: subscription sign-ins (Claude, ChatGPT) and their token
  renewal. `src/loopback/`: the port a browser sign-in comes back to.
- `src/completion.rs`: streaming chat requests (OpenAI-compatible, Anthropic, and the Responses API
  Codex speaks). `src/run.rs`: sending,
  streaming into the chat, stop and retry. `src/runs.rs`: what a run records.
  `src/prompt.rs`: what you send, your words and attachments (text files inlined, pictures shrunk to
  1568 px and sent as image parts). `src/composer.rs`: the message box's logic: `/` commands, `@` files
  from the working folder, ↑/↓ through what you sent, attaching (button, drop, paste), and a message
  sent while the agent answers waiting until the reply ends.
  `src/usage.rs`: usage totals, cost, and the Usage screen's report; `src/pricing.rs`: model prices from models.dev.
  `src/context.rs`: what the agent is told about the moment (the date and time), sent after its brief.
  `src/tools.rs`: the tools every agent is given (`ask_user_question`, `shell`, `read_file`, `write_file`, `edit_file`, `teach_workflow`, `run_workflow`).
  `src/workflow/`: workflows (definition and checks, references, the engine, learning one from a chat);
  `src/flows.rs`: teaching, saving and running them in the app.
  `src/mcp/`: MCP servers, any number of them. `client.rs` speaks JSON-RPC to a remote server (HTTP) or a local
  program (a child process, kept running between calls); `oauth.rs` signs in through the browser (discovery,
  registration, PKCE, a loopback callback, renewal); `flow.rs` is the form, the background work and the call;
  `catalog.rs` are the one-click presets. A server's tools join the register as `mcp__<server>__<tool>`, and an
  agent is given them in its form like any other. Keys, variables and tokens are sealed in the database
  (`store/mcp.rs`). Nothing asks before an MCP tool runs.
  `src/files.rs`: the file tools, confined to the agent's working folder.
  `src/command.rs`: running a shell command (runs at once when an agent that has the shell tool asks, in
  the agent's working folder). `src/folders.rs`: the folder browser behind that choice.
- `src/ui/`: the screens.

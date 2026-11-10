# Oido Desktop

A native desktop app for running agents, written in Rust on the [Rune](https://github.com/anomalyco/rune)
GPU UI framework. It is its own app: it does not talk to `oido-core`. It keeps its agents, chats,
providers and settings in a local SQLite database, and talks to model providers directly.

## Run

```
cargo run                      # opens the window
cargo watch -x run             # restarts on save (cargo install cargo-watch)
cargo run -- --snap out.png --scene providers --size 1280x900   # a headless render (debug builds; --scene NAME shows sample data)
```

Rune is pre-release, so `Cargo.toml` pins one revision of its `dev` branch. Bump it on purpose.

## Test

```
cargo test                     # unit and end-to-end tests; never touch your data or keyring
cargo test -- --ignored        # also exercises the real system keyring (needs a Secret Service)
cargo clippy --all-targets
```

## Release

```
scripts/release.sh 0.2.0       # bump Cargo.toml, test, build, tag v0.2.0, push, publish the GitHub release
oido-desktop --version
```

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
| Workflows | saved in the database with the agent that learned them (deleted with it); drafts wait in their chat; runs are kept 30 days |
| Model lists | fetched from each provider when it is connected, saved in the database, refreshed when stale |

A provider is connected only after its key has listed models: a key that cannot is not saved. If the
system keyring is unavailable, no key is stored.

## Layout

- `src/agents.rs`, `src/providers.rs`, `src/models.rs`: the domain (agents and teams, providers,
  models and what they can do).
- `src/store/`: SQLite, migrations and typed queries. `src/vault.rs`: sealing keys.
- `src/listing.rs`: asking a provider for its models. `src/connect.rs`: connecting, refreshing.
- `src/completion.rs`: streaming chat requests (OpenAI-compatible and Anthropic). `src/run.rs`: sending,
  streaming into the chat, stop and retry. `src/runs.rs`: what a run records.
  `src/prompt.rs`: what you send, your words and attachments (text files inlined, pictures shrunk to
  1568 px and sent as image parts). `src/composer.rs`: the message box's logic: `/` commands, `@` files
  from the working folder, ↑/↓ through what you sent, attaching (button, drop, paste), and a message
  sent while the agent answers waiting until the reply ends.
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

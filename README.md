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

## Where things live

| What | Where |
| --- | --- |
| Database | `~/.local/share/oido-desktop/oido.db` (override with `OIDO_DB`, `:memory:` for a throwaway) |
| Schema and queries | `db/migrations/*.sql` and `db/queries/*.sql`, run by `src/store` |
| Provider API keys | sealed in the database (XChaCha20-Poly1305); the key that opens them is in the OS keyring |
| Runs | each message you send is a run: the brief, your message, what was sent to the model and what came back (and any error), shown on the Runs screen. Kept 30 days, then deleted at startup; chats are never touched. Keys are never recorded |
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

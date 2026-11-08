# Oido Desktop

A native desktop app for running agents, written in Rust on the [Rune](https://github.com/anomalyco/rune)
GPU UI framework. It is its own app: it does not talk to `oido-core`. It keeps its agents, chats,
providers and settings in a local SQLite database, and talks to model providers directly.

## Run

```
cargo run                      # opens the window
cargo watch -x run             # restarts on save (cargo install cargo-watch)
cargo run -- --snap out.png --scene providers --size 1280x900   # a headless render
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
| Model lists | fetched from each provider when it is connected, saved in the database, refreshed when stale |

A provider is connected only after its key has listed models: a key that cannot is not saved. If the
system keyring is unavailable, no key is stored.

## Layout

- `src/agents.rs`, `src/providers.rs`, `src/models.rs`: the domain (agents and teams, providers,
  models and what they can do).
- `src/store/`: SQLite, migrations and typed queries. `src/vault.rs`: sealing keys.
- `src/listing.rs`: asking a provider for its models. `src/connect.rs`: connecting, refreshing.
- `src/ui/`: the screens.

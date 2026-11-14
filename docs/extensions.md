# Extensions

Extensions are tool sets we write and compile into Oido, for services that have no MCP server worth
using: Gmail first, then Google Calendar and Google Drive (Docs and Sheets). To an agent an
extension looks like an MCP server (`mcp__gmail__search`); to the person it is a switch on its own
screen. Nothing runs on our servers: the app talks straight to the service's API with the person's
own sign-in.

Status: design agreed 2026-11-14. P1 and P3 (Calendar, built early) are on `feat/extensions-p1`, uncommitted; P4 (Drive, built early, Drive API only) is there too; P2 (Gmail write) and Docs/Sheets editing not started.

## Decisions

**ADR-1: first-party and compiled in.** Only we write extensions, in Rust, one module each under
`src/extensions/`. No plugin format, no loading code at run time. Cost: a new extension needs a
release. Gain: no sandbox, no manifest, no versioning to design, and full use of the app's HTTP,
sealing and cancel plumbing. Revisit only if third parties ask.

**ADR-2: same register, `mcp__` names.** Extension tools are offered and called exactly as MCP
tools are, so agent forms, workflow tool steps, guardrails and the `mcp__<server>__<tool>` naming
need no second path. The seam is one new `Transport::Extension { id }` beside `Http` and `Stdio`
in `src/mcp/mod.rs`. `run.rs` already sends every `mcp::is_tool` name to `start_mcp_call`; that
function, and `client::call_tool`, branch on the transport and hand an `Extension` to
`extensions::call`. Rejected: a separate `ext__` namespace (a second picker, a second dispatch).

**ADR-3: an extension is a Rust trait object, with static tools.**

```rust
pub trait Extension: Sync {
    fn id(&self) -> &'static str;          // "gmail": the server name in `mcp__gmail__*`
    fn title(&self) -> &'static str;
    fn account(&self) -> Account;          // which sign-in it needs
    fn scopes(&self) -> &'static [&'static str];
    fn tools(&self) -> Vec<Tool>;          // name, description, JSON Schema
    fn call(&self, tool: &str, args: &Value, http: &Http) -> Result<String, String>;
}
```

`mcp::Tool` is reused so specs, summaries and the form-from-schema in workflow steps work as
they do for MCP. The list lives in `extensions::ALL`. Shared helpers (authorized request with one
refresh-and-retry, paging, response trimming, schema building) live in `extensions/http.rs` and
`extensions/schema.rs`, not in a macro and not in a declarative format: Gmail's MIME bodies and
Drive's export rules do not fit one.

**ADR-4: one Google account, one pasted client.** The person creates a Google Cloud OAuth client
(desktop type) and pastes its id and secret once, on the Extensions screen. One sign-in covers
Gmail, Calendar and Drive. Scopes are requested as the union of the *enabled* extensions' scopes;
enabling another extension asks Google for consent again (incremental authorization, same
refresh token family). Why their client, not ours: Gmail and Drive scopes are "restricted" and
need Google's security assessment for a shared app; a personal client in testing mode avoids that.
Cost: setup friction, and a testing-mode client's refresh tokens lapse after 7 days unless the
client is published to "In production" (unverified apps can do that for their owner's own use,
with a warning screen). The setup text says so.

**ADR-5: sign-in reuses the loopback and PKCE plumbing.** `src/loopback` receives the redirect;
`mcp/oauth.rs` already does PKCE, refresh and token types for a server URL. Google differs only in
fixed endpoints and a client secret, so `oauth.rs` gains a function that takes endpoints and client
instead of discovering them. One account, so one sign-in at a time, cancellable like MCP's.

**ADR-6: secrets sealed like provider and MCP secrets.** Client id, secret and tokens are one value
sealed with the vault (as `save_mcp_secrets` does), in a new `extension_accounts` row (`provider`
= `google`). Tokens refreshed during a call come back in the call's outcome and are saved on the UI
thread, because the vault cannot leave it; the same shape as `client::Outcome`. Secrets are never
printed (`Debug` redacts), and never written to the model's view.

**ADR-7: the toggle is the only gate.** An extension has an on/off switch, and each tool its own.
An off extension or tool is not offered to agents and not resolvable, so a saved agent that named it
just no longer has it (as with a removed MCP server). A tool that is on runs the moment the model
calls it, send and delete included, as MCP tools do; the existing guardrails apply on top. The Extensions screen labels each tool *reads* or *changes things* so the switches are informed.
Rejected for now: confirm-on-write. It can be added later on the same `reads/changes` label.

**ADR-8: results are plain text, cut to 32 KiB.** The existing `OUTPUT_LIMIT` applies. Tools return
what the model needs, not the raw API JSON: a Gmail message is headers, a plain-text body (HTML
stripped), attachment names and sizes; a Drive file is exported as text or Markdown. Search tools
page with a `page_token` argument and say when more exists. Attachments and large files are saved
into the agent's working folder and the path returned, never inlined.

**ADR-9: one account per extension family, no multi-account.** Switching accounts is sign out and
sign in. Keeps tool names stable (`mcp__gmail__search`) and the storage a single row.

**ADR-10: internal only.** Oido does not serve extensions to other MCP clients.

## Code

- `src/extensions/mod.rs`: the trait, `ALL`, `specs(&Store state)`, `call`. Which are on and which
  tools are off is `State` (below).
- `src/extensions/http.rs`: authorized requests, refresh-and-retry once on 401, backoff on 429/5xx,
  cancel checks, timeout (`CALL_TIMEOUT` of MCP).
- `src/extensions/google/`: `mod.rs` (the account: client, tokens, scopes in use), `gmail.rs`,
  `calendar.rs`, `drive.rs`.
- `src/mcp/mod.rs`: `Transport::Extension`; `specs` and `resolve` take extension tools in the same
  register. An extension appears as a read-only `Server` (not deletable, not editable) built from
  `State`, so everything that lists servers keeps working. Only an extension that is *on* is in
  the register (id `ext:<id>`, built by `App::rebuild_extension_servers`); the MCP page and its
  agent tool skip these (`Server::is_extension`), and the store refuses to save one.
- `src/store/extensions.rs`, `db/migrations/NNN_extensions.sql`, `db/queries/extensions.sql`:
  `extensions(id, enabled, off_tools)` and `extension_accounts(provider, sealed)`.
- `src/ui/extensions.rs`: the Extensions screen, in the rail beside MCP.

## The screen

A list of extensions: logo, name, one line, switch. Opening one shows:

1. **Account** (Google, shared): paste client id and secret, a link to the setup steps, *Sign in*
   (opens the browser, waits, cancellable), the signed-in address, *Sign out*. Until a client is
   saved the extension switches are disabled with the reason beside them.
2. **Tools**: each with its description, *reads* or *changes things*, and a switch.
3. **Status**: signed in, expired ("sign in again"), or the last failure in words.

## Failure modes

- Token revoked or refresh fails: the call returns "Google no longer accepts the sign-in. Ask the
  person to sign in again on the Extensions screen." and the screen shows *Sign in again*.
- Scope missing (extension enabled after sign-in): same message naming the extension.
- Quota or rate limit: backoff, then a plain message with the retry time if given.
- Offline: the call fails fast with the network error; nothing is queued.
- Cancel (Stop): the request's cancel flag ends the wait; a half-done write is reported as such,
  never retried automatically (sends are not idempotent).
- Testing-mode client expiry (7 days): detected by `invalid_grant`, same sign-in-again path.

## Phases

**P1: spine and Gmail read.** `Transport::Extension`, the trait, `State` and store, the Google
account (paste client, sign-in, refresh, sign-out), the Extensions screen, Gmail tools
`search`, `read_message`, `read_thread`, `list_labels`. Proof: unit tests on the trait, schemas,
scope union, and trimming with a fake `Http`; a sealed-secrets round trip; one live run against a
real mailbox.

**P2: Gmail write.** `create_draft`, `send`, `reply`, `modify_labels`, `archive`, `trash`, plus
attachment save. MIME building lives in `google/mime.rs` with tests.

**P3: Calendar (built).** Title "Google Calendar", tools `mcp__google_calendar__*`, scopes `calendar.events` and `calendar.readonly`. Writes are never retried. `list_events`, `get_event`, `create_event`, `update_event`, `delete_event`,
`free_busy`. Incremental scope consent exercised for the first time.

**P4: Drive (built, Drive API only).** Title "Google Drive", tools `mcp__google_drive__*`: `search_files`, `get_file`, `read_file` (Docs and Slides as text, Sheets as CSV, text files as they are), `create_file` (text file or Google Doc), `trash_file`. Scopes `drive.readonly` and `drive.file`: it reads everything but changes only files Oido made. Editing Docs and Sheets in place needs their own APIs and is not built. Originally planned: `search_files`, `read_file` (export to text), `read_doc`,
`read_sheet`, `append_rows`, `update_cells`, `create_doc`.

P1 as built: an extension that is on is offered to agents at once, even before the account is signed
in, so tools can be given to agents first; the page says under the switch what is missing, and a
call before then fails in words ("sign in again").

Each phase ends with `cargo fmt`, clippy with `-D warnings`, tests, and a snap render of the
Extensions screen run with `OIDO_DB=:memory:`.

## Open risks

- Google's consent screen and token lifetime rules change; the setup text must stay current.
- A pasted client secret is as sensitive as a password; it is sealed but is on this computer.
- Tool names are a contract with saved agents and workflows: renaming one later breaks them.
  Names are chosen once, carefully.

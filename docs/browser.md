# Browser

An agent can use the person's own browser (Chrome, Brave, Edge or any Chromium) to open pages, read
them, click and type, signed in as the person. It works through the **Oido Browser Connector**,
the extension oido-studio already uses (`oido-std/oido-browser-connector`). Desktop takes the place
of the cloud hub: the extension's base URL is `http://127.0.0.1:47821`, and its token comes from
Oido's Settings → Browser. Nothing runs on our servers.

Status: built 2026-11-15 on `feat/browser-p1` (`src/connector/`). It is tested end to end against
the real extension in headless Chromium
(`OIDO_CONNECTOR_DIR=…/oido-browser-connector cargo test the_real_connector -- --ignored`).

## Decisions

**ADR-1: reuse the Oido Browser Connector; Desktop is its hub.** The extension already owns the
hard parts: tabs of the agent's own in a window of their own, real input over `chrome.debugger`,
reads across frames, the per-origin allowlist, the per-tab origin lock, the Stop pill and ad
blocking. Desktop serves the same two endpoints oido-core serves
(`internal/server/handlers/browser_agent.go`):

- `GET /api/browser/stream`: server-sent events, one `data: {"id","action","params"}` frame per
  command. A newer stream replaces the old one, and an idle stream sends `: ping` every 15 s.
- `POST /api/browser/result`: `{"id","data","error"}`, answered `{"accepted":true}`.

Both need `Authorization: Bearer <token>`. The extension's Options already accept an `http://`
base on `127.0.0.1`, so it works unchanged. Rejected: driving Chrome over remote debugging (a first
build did). It needed `chrome://inspect/#remote-debugging` turned on (it's `brave://` in Brave), a
consent prompt each run, and our own allowlist and input code.

**ADR-2: a fixed port, a sealed token.** The hub listens on `127.0.0.1:47821` from launch, so the
base URL never changes. Headless renders and tests don't listen. The token is 24 random bytes in
hex, sealed in the database with the vault (setting `connector.token`). It is made on first launch,
copied from Settings, and replaced by **New token** (the old one stops working; a stream already
open keeps going until the extension reconnects). If the port is taken, Settings says so.

**ADR-3: one `browser` tool, as in oido-core.** Its schema and description follow oido-core's
`internal/tools/browser.go`, and so does the wire format the extension expects. The model gets one
`action` and refs from the latest `read` (`c3` controls, `l1` links), never selectors. Ported:
ref resolution (a click on a link opens it; choose and check swap for the right question kind; a
click on a radio group is refused), the read rendering (`renderRead`, `controlLine`), the extra
read after landing on a page, and the "lost result" guard (a click whose answer never came is not
repeated blind). Not ported yet: batches (`actions`), upload, screenshot, TypeSafe's filtering
and send checks, and site recordings. The tool is chosen per agent (group "Browser"), not given to
every agent. Team members and workflow steps don't get it.

**ADR-4: the extension decides which sites.** Its allowlist and "Allow all domains" are the only
site rule; Desktop keeps no list of its own. The first build's per-agent list (migration 025) was
dropped before release: 025 is now an empty step, and databases that ran it keep an unused
`agents.browse` column.

**ADR-5: approvals are the one Desktop setting.** Settings → Browser, saved as `browser.approvals`
for every agent:

| Row | Actions | Default |
|---|---|---|
| Opening pages | navigate, back, forward, reload | Never ask |
| Clicking and typing | click, type, press, submit, choose, check, set, hover, drag, dialog | Ask after reading a page |

The choices are Never ask, Ask after reading a page, and Always ask. Reading (read, tabs, scroll,
console, network, close) never asks. "After reading a page" is the guardrails taint rule: every
`browser` result is outside text ("a web page"), so while one is in what the model was sent, an
acting call waits for a go-ahead. It applies only with that pack on. `Guard::review` asks
`connector::kind` and `Approvals::check`, and cards, waits and timeouts are the usual ones.

**ADR-6: one action at a time.** The extension's tabs are shared, so calls from different chats
take turns. Refs belong to the chat whose read made them.

## Failure modes

- No extension connected: the model is told where to get it, what base URL and token to paste, and
  to retry. A connector that dropped within the last minute is waited for up to 10 s.
- It drops mid-action, or doesn't answer in 60 s: the model is told the action may have happened
  and to read before repeating. A repeat of that same mutating call is refused until it reads.
- Port 47821 taken: Settings → Browser says so.

## Next

- Batches (`actions`), screenshots into the chat, upload from the working folder.
- A setup card with a link to the store listing.
- TypeSafe send checks before consequential submits, when TypeSafe is connected.

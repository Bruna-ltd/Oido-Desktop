# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

<!-- Impeccable's platform field has no "native desktop" value; `web` is the closest default and loads no mobile guidance. The truth: a native Linux desktop app (Rust on the Rune GPU UI framework, rune-kit components), not a website. Linux today, with Omarchy/Hyprland as the home desktop; macOS and Windows are wanted later but undecided. -->

## Users

A solo builder or developer running their own agents on their own machine: coding, automation and ops work they would otherwise script by hand or hand to a hosted assistant. Technical, comfortable with keys, shells, folders and MCP servers, and wanting control over what an agent does and what it costs.

## Product Purpose

Oido Desktop is a native app for running agents locally. You bring your own provider keys or subscriptions; Oido keeps agents, chats, workflows and settings in a local SQLite database and talks to model providers directly. It does not talk to `oido-core` and runs no Oido server.

Success: the user's repeated agent work gets done reliably and cheaply, because work that has been done once becomes a reusable workflow instead of a fresh, token-hungry conversation.

## Positioning

- **Workflows that cut token use.** An agent learns a saved workflow from a chat, and a matching saved workflow runs first, deterministically, instead of the model reasoning it out again. The script/workflow builder can plan the actions ahead and write them as a script (Rhai code steps, branches, loops) so it runs again without the model. This is the biggest advantage.
- **Local-first, your own keys.** Data lives in a local database; provider keys are sealed (XChaCha20-Poly1305) with the opening key in the OS keyring; calls go straight to providers.

## Operating Context

- Desktop window plus a tray icon; closing the window leaves Oido running in the tray so schedules fire and channel bots keep answering. Desktop notifications when a reply finishes while the window is hidden.
- Agents work inside a chosen working folder with shell and file tools.
- Agents also reach the user through channels (Telegram, Slack, WhatsApp), on schedules, and through the Oido Browser Connector extension.
- Each message is a run, recorded on the Runs screen (setup, model calls, tool calls, timing); usage and estimated cost on the Usage screen.

## Capabilities and Constraints

- Agents and teams (a lead hands work to its members), providers and models (API keys; Claude Pro/Max and ChatGPT-plan Codex sign-ins), MCP servers, built-in extensions (Google Workspace, YouTube, GA4, Search Console), skills, browser tool.
- Workflows: teach, save, edit (tree and graph views), run; Rhai expressions and code steps with no file, network or process access.
- Guardrails (both directions: secrets hidden from models, risky actions held for a go-ahead, blocked places), user lifecycle hooks, context-window meter and trimming.
- Themes: Oido's own monochrome dark look, or the user's Omarchy theme colours, on rune-kit's semantic tokens.
- Linux only for now; needs OpenSSL 3. Self-updates from GitHub releases.
- Undecided: macOS/Windows timing.

## Evidence on Hand

- App icon: `assets/icon/oido-desktop.svg`, `.png`. Agent avatars, provider, channel and MCP logos under `assets/`. Lucide icons (`LICENSE-lucide`).
- Feature docs in `docs/` (browser, channels, context, extensions, guardrails, hooks, scheduler, usage, workflow builder).
- No testimonials, user counts, benchmarks or measured token-savings figures exist; do not invent them.

## Product Principles

1. Done once, reused cheaply: prefer a saved workflow or script over another model call.
2. Local and owned: the user's data, keys and costs stay visible and on their machine.
3. Nothing silent: what an agent did, cost and was allowed to do is always inspectable.
4. Built for someone technical: precise and dense over hand-holding.

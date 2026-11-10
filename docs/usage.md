# Usage

A Usage screen shows what agents spend, in tokens and an estimated cost in US dollars: over time,
by provider and model, by agent, and by source (chat, schedule, workflow, team member). It also
shows how well the prompt cache works. Runs, the chat header and the agent form show cost too.

Status: built 2026-11-10.

## What it does

**The Usage screen** (rail → Usage) shows the following:

- A range: 7 days, 30 days, 90 days, or all.
- Tiles: cost, tokens, model calls, cache hit rate (cache reads ÷ input), average time to first
  token, and error rate.
- A daily bar chart that switches between cost and tokens.
- Tables by provider/model, by agent, and by source. Clicking a row narrows the whole screen to
  it. Chips show the active filters and clear them.
- Models with no known price show "—" for cost, plus a **Set price** action. It opens an inline
  form (US$ per million tokens for input, output, cache read and cache write). A manual price
  overrides the catalog one.

**Elsewhere**:

- Runs screen: the cost of each run and of each model call, next to its tokens.
- Chat header: the tokens and cost for this chat so far.
- Agent form: what the agent spent in the last 30 days.
- Settings: **Clear usage history**, after a confirmation. It empties the daily totals. Chat
  totals and runs are kept.

Days are in local time. Cost is always an estimate, and the screen says so.

## Data

Migration `012_usage.sql` makes these changes:

- `runs` and `run_steps` get `cache_read_tokens` and `cache_write_tokens`. `input_tokens` still
  counts all input, cached or not.
- `usage_daily` holds one row per `(day, provider, model, agent_id, source)`:
  - The agent's name as it was.
  - Totals: calls, input, output, cache read, cache write, errors.
  - Time to first token, kept as a sum and a count.
  - The 30-day run deletion never touches it. Existing model-call steps are backfilled into it,
    without a cache split. Source comes from `runs.parent` (team member) and `schedule_fires`
    (schedule).
- `session_usage` holds one row per `(session_id, provider, model)` with the same token totals.
  It is deleted with its chat.
- `model_prices` holds one row per `(provider, model, origin)`, where origin is `catalog` or
  `manual`. It stores US$ per million tokens for input, output, cache read and cache write, plus
  `fetched_at`. A manual row wins.

## How it works

- **Capture.** `Event::Usage` carries cache read and cache write as well:
  - Anthropic reports `cache_read_input_tokens` and `cache_creation_input_tokens`.
  - OpenAI-compatible providers report `prompt_tokens_details.cached_tokens`.

  Every model call is counted in one place, at the moment its step is recorded. That is a chat
  turn, a team member's turn, or a workflow engine call (under the agent's model, source
  `workflow`). It adds to `usage_daily` and `session_usage` in the same transaction as the step.
  Counting per call, not per run, keeps a team member's tokens from being counted twice. Its
  parent's run totals include them.
- **Prices.** `src/pricing.rs` fetches `https://models.dev/api.json` in the background, at launch
  and then once a day. It maps provider ids (`gemini`→`google`, `kimi`→`moonshotai`,
  `qwen`→`alibaba`, the rest unchanged) and replaces the `catalog` rows. If the fetch fails, the
  last prices stay. Nothing about the user is sent.
- **Cost** is worked out when shown, never stored:
  `(input − cache_read − cache_write)·in + cache_read·cache_read_price + cache_write·cache_write_price + output·out`.
  A missing cache price falls back to the input price.

## Decisions

| Decision | Chosen | Instead of | Because | Cost |
| --- | --- | --- | --- | --- |
| Price source | models.dev catalog, plus manual override | Prices bundled in the binary; user-entered only | Current prices for most providers, with no release needed | Needs network; ids need mapping; local and custom models need a manual price |
| When cost is computed | At view time | A snapshot when the run ends | A price added later fixes old rows; nothing is frozen at $0 | Past cost moves if a price changes |
| History | Daily totals kept forever | Querying runs (30 days); keeping runs longer | Small table, and the history is unbounded | A second write per model call |
| Counting unit | Each model call | Each run's totals | Each call belongs to exactly one run, so team members are not counted twice | Workflow calls need their own hook |
| Chat totals | `session_usage` per chat and model | Summing the runs that are left | Survives run deletion; prices by model | One more table |

## Phases

1. Data, capture and pricing: migration, cache split, the counting hook (chat, team member,
   workflow), backfill, models.dev fetch, cost function. Tests.
2. Usage screen: range, tiles, chart, tables, filters, Set price.
3. Elsewhere: cost on Runs, chat header total, agent form spend, Clear usage history.

## Not included

Budgets and alerts, CSV export, TypeSafe calls (the shape of their usage is not known yet),
currencies other than US$.

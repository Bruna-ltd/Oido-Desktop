# Context window

Every model takes only so many tokens at once: the brief, skills, tools, the chat and the reply
together. A long chat, a long run of tool calls or a big paste can go past that. This is how the
app knows how full a chat is, warns before it is full, and makes room on its own instead of
failing.

Status: agreed and built 2026-11-10.

## Before

- Every call sent the whole chat since the last "Start fresh" divider, with every tool result and
  picture, and nothing measured it.
- A provider that refused for length gave `Error::TooLong`: "This chat is too long for the model.
  Press Start fresh, then send it again." Start fresh made the model forget the whole chat.
- A run of tool calls could hit the limit midway and fail with its work unfinished.
- llama-server's refusal ("the request exceeds the available context size") matched none of the
  known phrases, so it showed as a plain HTTP 400.

## What it does

**Phase 1: know the limit, show the fill.**

- Each model has a context limit when something states it, else none (unknown, never guessed):
  1. What the provider's own list declares: `context_length` (OpenRouter and others),
     `max_model_len` (vLLM), `max_context_length` (Mistral), `inputTokenLimit` (Gemini).
  2. For your own server, what llama-server's `/props` says it runs with
     (`default_generation_settings.n_ctx`). A server without `/props` is fine.
  3. models.dev's `limit.context`, fetched with the prices, matched as prices are (exact id, then
     the id without its date or variant).
- How full a chat is comes from the provider's own count: each reply keeps the tokens of its last
  call (input, cached input included, plus output) as `Stats::context`. The chat's fill is the
  last finished reply's count since the last divider.
- The chat header shows it beside the spend: "62% context" with the limit, or "41k context" when
  the limit is unknown. At 80% and above it turns amber. Start fresh clears it.
- llama-server's "context size" refusal counts as too long.

**Phase 2: make room without a summary.** Before every model call, in chat and in a run alike:

- The size is the last measured count plus about one token per four characters for what came
  after it.
- Past 90% of the limit, the turns sent are trimmed, oldest first, until they fit under 75%. The
  last four turns are never trimmed.
  - A tool result over 1 KB becomes `(output removed to save room: 41 KB)`. The call stays, so the
    model knows what it did.
  - Pictures become `[picture removed to save room]`, counted as 1,600 tokens each.
- What was trimmed stays trimmed on later calls until the chat no longer fits under 90%, and only
  then is more trimmed. Each reply keeps the estimated tokens trimmed as `Stats::trimmed`, so the
  next call knows the full size (the count plus what was trimmed) and trims the same turns again.
  That keeps what is sent the same from call to call, so the provider's cache keeps working.
- Only what is sent is trimmed. The chat on screen and in the database keeps everything.
- A too-long refusal goes again at once with everything trimmable trimmed, even when the limit
  is unknown. This happens once, and only if there is something to trim. The run shows the
  refused call, an error step, then the second call.
- Chat runs (`run.rs`) and team members (`delegate.rs`) both go through `window::fit`. Workflow
  steps send a single message and are not trimmed.

**Phase 3: summarize.** When trimming is not enough:

- If a call is still past 90% of a known limit after trimming, or the provider refuses it as too
  long even after the trimmed retry, one call to the same model summarizes everything before the
  last four turns. It keeps what you asked for, decisions, facts, file paths, what the tools did
  and what is still open. Each message is cut to 4 KB for it, and it is given at most 60% of the
  limit (200 KB when the limit is unknown), keeping the most recent part.
- The summary is added to the end of the chat where it happened, like a divider, as
  `Message::Summary { text, keeps }`. The model is then sent the summary, the `keeps` messages
  right above it word for word, and what follows. Nothing earlier in the chat moves.
- On screen it is a marker, "Earlier messages summarized to make room", that opens to the summary
  in a muted card. The messages above it stay on screen.
- It is part of the same run: the run shows the summary as a model call named `summary`, then
  the reply. Usage counts it under its own source, `compaction` ("Summaries"). Migration
  `014_context.sql` rebuilds `usage_daily` so its CHECK allows that source.
- A count from before a summary no longer measures the chat, so the next call is estimated fresh
  and the header meter clears until the next reply.
- The chat shows as working while the summary is written, and Stop ends it.
- If the summary call fails, the reply fails with "Could not summarize the earlier messages to
  make room: …".
- If there is nothing left to summarize (fewer than four turns since the last summary) and the
  call still cannot fit, it fails before anything is sent, with the sizes: "…it would be sent
  about 150.2k tokens and takes 128.0k. Shorten your last message, or press Start fresh."
- Team members are trimmed but not summarized. Their work is one task, and a task too big even
  trimmed fails as before.

## Decisions

- **Make room on its own, cheapest first.** We did not take a sliding window, which drops early
  instructions without a word. We did not take "Start fresh only", which leaves the work to you
  and still lets runs fail midway.
- **Count with the provider's numbers, not a tokenizer.** A tokenizer for every provider is
  heavy and impossible for unknown local models. The last call's count is exact, and the ÷4
  estimate only covers what came after it, within a 15% margin.
- **Summarize automatically,** with a visible marker and no question first, so a run does not
  stop to wait for you.
- Trimming or summarizing changes what was sent before, so the provider's prompt cache misses
  once afterwards. That is rare and accepted.

## Risks

- A summary can drop a detail. The last turns go word for word, the prompt says what to keep,
  and the summary is visible.
- A trimmed tool result may make the model run the tool again.
- Some local servers cut the chat silently instead of refusing. That cannot be seen: such a model
  shows "41k context" with no limit.
- `Message::Summary` is a new kind of saved message. Older builds cannot read a chat that has
  one.
- If you send a message while a summary is being written, it waits and goes after the reply, as
  it does for any reply in progress.

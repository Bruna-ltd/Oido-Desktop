-- Usage: what agents spend, in tokens, kept as daily totals that outlive the runs they came from,
-- and the prices that turn tokens into an estimated cost. See docs/usage.md.

-- Cache reads and writes, as providers report them. Both are part of input_tokens.
ALTER TABLE runs ADD COLUMN cache_read_tokens INTEGER;
ALTER TABLE runs ADD COLUMN cache_write_tokens INTEGER;
ALTER TABLE run_steps ADD COLUMN cache_read_tokens INTEGER;
ALTER TABLE run_steps ADD COLUMN cache_write_tokens INTEGER;

-- One row per local day, model, agent and source, added to as each model call ends. Kept until the
-- user clears it: run retention never touches it, and it keeps no link to the agent (it outlives it).
-- source: what made the call — a chat, a schedule's chat, a workflow, or a team member's turn.
CREATE TABLE usage_daily (
    day                TEXT NOT NULL,
    provider           TEXT NOT NULL,
    model              TEXT NOT NULL,
    agent_id           TEXT NOT NULL,
    agent_name         TEXT NOT NULL,
    source             TEXT NOT NULL CHECK (source IN ('chat', 'schedule', 'workflow', 'member')),
    calls              INTEGER NOT NULL DEFAULT 0,
    errors             INTEGER NOT NULL DEFAULT 0,
    input_tokens       INTEGER NOT NULL DEFAULT 0,
    output_tokens      INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens  INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    -- Time to first token: a sum and how many calls it covers, for an average.
    first_ms_total     INTEGER NOT NULL DEFAULT 0,
    first_ms_count     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, provider, model, agent_id, source)
) STRICT, WITHOUT ROWID;

-- A chat's tokens by model, so its total survives run retention. Goes with the chat.
CREATE TABLE session_usage (
    session_id         TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    provider           TEXT NOT NULL,
    model              TEXT NOT NULL,
    input_tokens       INTEGER NOT NULL DEFAULT 0,
    output_tokens      INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens  INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, provider, model)
) STRICT, WITHOUT ROWID;

-- US dollars per million tokens. 'catalog' rows come from models.dev and are replaced on each
-- fetch; a 'manual' row is the user's and wins. A missing cache price is charged as input.
CREATE TABLE model_prices (
    provider    TEXT NOT NULL,
    model       TEXT NOT NULL,
    origin      TEXT NOT NULL CHECK (origin IN ('catalog', 'manual')),
    input       REAL NOT NULL CHECK (input >= 0),
    output      REAL NOT NULL CHECK (output >= 0),
    cache_read  REAL CHECK (cache_read >= 0),
    cache_write REAL CHECK (cache_write >= 0),
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (provider, model, origin)
) STRICT, WITHOUT ROWID;

-- The runs still kept count from the start. Workflow runs recorded no model, so they are not.
INSERT INTO usage_daily (day, provider, model, agent_id, agent_name, source, calls, errors,
                         input_tokens, output_tokens, first_ms_total, first_ms_count)
SELECT date(s.started_at, 'unixepoch', 'localtime'), r.model_provider, r.model, r.agent_id,
       MAX(r.agent_name),
       CASE WHEN r.parent_run_id IS NOT NULL THEN 'member'
            WHEN EXISTS (SELECT 1 FROM schedule_fires f WHERE f.session_id = r.session_id) THEN 'schedule'
            ELSE 'chat' END AS src,
       COUNT(*), SUM(s.status = 'error'),
       SUM(COALESCE(s.input_tokens, 0)), SUM(COALESCE(s.output_tokens, 0)),
       SUM(COALESCE(s.first_ms, 0)), COUNT(s.first_ms)
FROM run_steps s JOIN runs r ON r.id = s.run_id
WHERE s.kind = 'llm'
GROUP BY 1, 2, 3, 4, src;

INSERT INTO session_usage (session_id, provider, model, input_tokens, output_tokens)
SELECT r.session_id, r.model_provider, r.model,
       SUM(COALESCE(s.input_tokens, 0)), SUM(COALESCE(s.output_tokens, 0))
FROM run_steps s JOIN runs r ON r.id = s.run_id
WHERE s.kind = 'llm' AND r.session_id IN (SELECT id FROM sessions)
GROUP BY 1, 2, 3;

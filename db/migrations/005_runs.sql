-- Runs: one message and the model's answer to it, with the steps inside (the brief, your message,
-- the model call, an error). Modelled on oido-core's agent_runs and agent_run_steps. A run keeps no
-- link to its agent or chat: it is a record, and outlives both. Removed after 30 days by the app.
CREATE TABLE runs (
    id            TEXT PRIMARY KEY,
    agent_id      TEXT NOT NULL,
    agent_name    TEXT NOT NULL,
    session_id    TEXT NOT NULL,
    model_provider TEXT NOT NULL,
    model         TEXT NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('running', 'ok', 'error', 'cancelled')),
    error         TEXT,
    input_tokens  INTEGER,
    output_tokens INTEGER,
    duration_ms   INTEGER,
    started_at    INTEGER NOT NULL,
    ended_at      INTEGER
) STRICT;

CREATE INDEX runs_by_time ON runs (started_at DESC);
CREATE INDEX runs_by_session ON runs (session_id, started_at DESC);

-- Written once the run ends, in order. (run_id, seq) is the natural key.
CREATE TABLE run_steps (
    run_id        TEXT NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
    seq           INTEGER NOT NULL,
    kind          TEXT NOT NULL CHECK (kind IN ('system', 'user', 'llm', 'error')),
    name          TEXT NOT NULL DEFAULT '',
    status        TEXT NOT NULL DEFAULT 'ok' CHECK (status IN ('ok', 'error', 'cancelled')),
    input         TEXT,
    output        TEXT,
    reasoning     TEXT,
    input_tokens  INTEGER,
    output_tokens INTEGER,
    duration_ms   INTEGER,
    started_at    INTEGER NOT NULL,
    truncated     INTEGER NOT NULL DEFAULT 0 CHECK (truncated IN (0, 1)),
    PRIMARY KEY (run_id, seq)
) STRICT, WITHOUT ROWID;

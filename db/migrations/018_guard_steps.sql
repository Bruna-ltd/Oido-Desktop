-- A run's steps can be a guardrail at work: secrets hidden from the model, or a tool call refused,
-- held for the person's go-ahead, then allowed or not. SQLite cannot change a CHECK, so the table
-- is copied into one that allows 'guard'.
CREATE TABLE run_steps_new (
    run_id             TEXT NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
    seq                INTEGER NOT NULL,
    kind               TEXT NOT NULL CHECK (kind IN ('system', 'skill', 'context', 'tools', 'user', 'llm', 'tool', 'guard', 'error')),
    name               TEXT NOT NULL DEFAULT '',
    status             TEXT NOT NULL DEFAULT 'ok' CHECK (status IN ('ok', 'error', 'cancelled')),
    input              TEXT,
    output             TEXT,
    reasoning          TEXT,
    input_tokens       INTEGER,
    output_tokens      INTEGER,
    duration_ms        INTEGER,
    started_at         INTEGER NOT NULL,
    at_ms              INTEGER,
    first_ms           INTEGER,
    truncated          INTEGER NOT NULL DEFAULT 0 CHECK (truncated IN (0, 1)),
    cache_read_tokens  INTEGER,
    cache_write_tokens INTEGER,
    PRIMARY KEY (run_id, seq)
) STRICT, WITHOUT ROWID;

INSERT INTO run_steps_new (run_id, seq, kind, name, status, input, output, reasoning,
                           input_tokens, output_tokens, duration_ms, started_at, at_ms, first_ms,
                           truncated, cache_read_tokens, cache_write_tokens)
SELECT run_id, seq, kind, name, status, input, output, reasoning,
       input_tokens, output_tokens, duration_ms, started_at, at_ms, first_ms,
       truncated, cache_read_tokens, cache_write_tokens
FROM run_steps;

DROP TABLE run_steps;
ALTER TABLE run_steps_new RENAME TO run_steps;

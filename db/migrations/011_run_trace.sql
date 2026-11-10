-- A run now lasts from what you said until the agent stops or waits on you: every model call and
-- tool use in between is one of its steps, and so is each skill, the context and the tools on offer.
-- Steps say when they began within the run (at_ms) and, for a model call, how long until its reply
-- began (first_ms). A run is named after the first line of what you said.
ALTER TABLE runs ADD COLUMN title TEXT NOT NULL DEFAULT '';

CREATE TABLE run_steps_new (
    run_id        TEXT NOT NULL REFERENCES runs (id) ON DELETE CASCADE,
    seq           INTEGER NOT NULL,
    kind          TEXT NOT NULL CHECK (kind IN ('system', 'skill', 'context', 'tools', 'user', 'llm', 'tool', 'error')),
    name          TEXT NOT NULL DEFAULT '',
    status        TEXT NOT NULL DEFAULT 'ok' CHECK (status IN ('ok', 'error', 'cancelled')),
    input         TEXT,
    output        TEXT,
    reasoning     TEXT,
    input_tokens  INTEGER,
    output_tokens INTEGER,
    duration_ms   INTEGER,
    started_at    INTEGER NOT NULL,
    at_ms         INTEGER,
    first_ms      INTEGER,
    truncated     INTEGER NOT NULL DEFAULT 0 CHECK (truncated IN (0, 1)),
    PRIMARY KEY (run_id, seq)
) STRICT, WITHOUT ROWID;

INSERT INTO run_steps_new (run_id, seq, kind, name, status, input, output, reasoning,
                           input_tokens, output_tokens, duration_ms, started_at, truncated)
SELECT run_id, seq, kind, name, status, input, output, reasoning,
       input_tokens, output_tokens, duration_ms, started_at, truncated
FROM run_steps;

DROP TABLE run_steps;
ALTER TABLE run_steps_new RENAME TO run_steps;

-- Runs from before are named after their message's first line too.
UPDATE runs SET title = COALESCE((
    SELECT substr(trim(CASE WHEN instr(output, char(10)) > 0
                            THEN substr(output, 1, instr(output, char(10)) - 1)
                            ELSE output END), 1, 80)
    FROM run_steps s WHERE s.run_id = runs.id AND s.kind = 'user' ORDER BY s.seq LIMIT 1), '');

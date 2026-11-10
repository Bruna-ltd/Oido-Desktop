-- How many tokens a model takes at once, and summaries written to make room (see docs/context.md).

-- What a provider's own list declares, or what a server says it runs with.
ALTER TABLE models ADD COLUMN context INTEGER;

-- models.dev's limits, fetched with its prices and replaced with them.
CREATE TABLE model_limits (
    provider TEXT NOT NULL,
    model    TEXT NOT NULL,
    context  INTEGER NOT NULL CHECK (context > 0),
    PRIMARY KEY (provider, model)
) STRICT, WITHOUT ROWID;

-- The catalog and the lists are asked again soon, so the limits fill in. What is saved stays
-- in use until then.
UPDATE model_prices SET updated_at = 0 WHERE origin = 'catalog';
UPDATE providers SET models_fetched_at = 0 WHERE models_fetched_at IS NOT NULL;

-- A summary written to make room is counted under its own source. SQLite cannot change a CHECK,
-- so the table is rebuilt with the same rows.
CREATE TABLE usage_daily_new (
    day                TEXT NOT NULL,
    provider           TEXT NOT NULL,
    model              TEXT NOT NULL,
    agent_id           TEXT NOT NULL,
    agent_name         TEXT NOT NULL,
    source             TEXT NOT NULL
        CHECK (source IN ('chat', 'schedule', 'workflow', 'member', 'compaction')),
    calls              INTEGER NOT NULL DEFAULT 0,
    errors             INTEGER NOT NULL DEFAULT 0,
    input_tokens       INTEGER NOT NULL DEFAULT 0,
    output_tokens      INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens  INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    first_ms_total     INTEGER NOT NULL DEFAULT 0,
    first_ms_count     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, provider, model, agent_id, source)
) STRICT, WITHOUT ROWID;
INSERT INTO usage_daily_new SELECT * FROM usage_daily;
DROP TABLE usage_daily;
ALTER TABLE usage_daily_new RENAME TO usage_daily;

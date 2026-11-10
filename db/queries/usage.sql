-- Adds model calls to a day's totals. ?1 is when they happened (seconds since the epoch), counted
-- on the local day.
-- name: AddUsage :exec
INSERT INTO usage_daily (day, provider, model, agent_id, agent_name, source, calls, errors,
                         input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                         first_ms_total, first_ms_count)
VALUES (date(?1, 'unixepoch', 'localtime'), ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
ON CONFLICT (day, provider, model, agent_id, source) DO UPDATE SET
    agent_name         = excluded.agent_name,
    calls              = calls + excluded.calls,
    errors             = errors + excluded.errors,
    input_tokens       = input_tokens + excluded.input_tokens,
    output_tokens      = output_tokens + excluded.output_tokens,
    cache_read_tokens  = cache_read_tokens + excluded.cache_read_tokens,
    cache_write_tokens = cache_write_tokens + excluded.cache_write_tokens,
    first_ms_total     = first_ms_total + excluded.first_ms_total,
    first_ms_count     = first_ms_count + excluded.first_ms_count;

-- Adds to a chat's totals; nothing when the chat is not saved.
-- name: AddSessionUsage :exec
INSERT INTO session_usage (session_id, provider, model, input_tokens, output_tokens,
                           cache_read_tokens, cache_write_tokens)
SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 WHERE EXISTS (SELECT 1 FROM sessions WHERE id = ?1)
ON CONFLICT (session_id, provider, model) DO UPDATE SET
    input_tokens       = input_tokens + excluded.input_tokens,
    output_tokens      = output_tokens + excluded.output_tokens,
    cache_read_tokens  = cache_read_tokens + excluded.cache_read_tokens,
    cache_write_tokens = cache_write_tokens + excluded.cache_write_tokens;

-- Whether a schedule opened the chat.
-- name: IsScheduledSession :one
SELECT EXISTS (SELECT 1 FROM schedule_fires WHERE session_id = ?1);

-- name: ListSessionUsage :many
SELECT provider, model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens
FROM session_usage WHERE session_id = ?1;

-- name: ListAllSessionUsage :many
SELECT provider, model, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, session_id
FROM session_usage;

-- Daily totals from ?1 (a 'YYYY-MM-DD' day; '' for all), oldest first.
-- name: ListUsage :many
SELECT day, provider, model, agent_id, agent_name, source, calls, errors, input_tokens,
       output_tokens, cache_read_tokens, cache_write_tokens, first_ms_total, first_ms_count
FROM usage_daily WHERE day >= ?1 ORDER BY day;

-- name: ClearUsage :exec
DELETE FROM usage_daily;

-- Every price, the user's after the catalog's so that theirs is read last and wins.
-- name: ListPrices :many
SELECT provider, model, origin, input, output, cache_read, cache_write
FROM model_prices ORDER BY origin = 'manual';

-- name: PutPrice :exec
INSERT INTO model_prices (provider, model, origin, input, output, cache_read, cache_write, updated_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
ON CONFLICT (provider, model, origin) DO UPDATE SET
    input = excluded.input, output = excluded.output, cache_read = excluded.cache_read,
    cache_write = excluded.cache_write, updated_at = excluded.updated_at;

-- name: DeleteCatalogPrices :exec
DELETE FROM model_prices WHERE origin = 'catalog';

-- name: ListLimits :many
SELECT provider, model, context FROM model_limits;

-- name: DeleteLimits :exec
DELETE FROM model_limits;

-- name: PutLimit :exec
INSERT OR REPLACE INTO model_limits (provider, model, context) VALUES (?1, ?2, ?3);

-- name: DeleteManualPrice :exec
DELETE FROM model_prices WHERE provider = ?1 AND model = ?2 AND origin = 'manual';

-- When the catalog was last fetched; NULL if never.
-- name: CatalogFetchedAt :one
SELECT MAX(updated_at) FROM model_prices WHERE origin = 'catalog';

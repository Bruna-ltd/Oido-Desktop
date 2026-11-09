-- name: InsertRun :exec
INSERT INTO runs (id, agent_id, agent_name, session_id, model_provider, model, status, started_at, parent_run_id)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', ?7, ?8);

-- name: FinishRun :exec
UPDATE runs SET status = ?2, error = ?3, input_tokens = ?4, output_tokens = ?5, duration_ms = ?6, ended_at = ?7
WHERE id = ?1;

-- name: InsertStep :exec
INSERT INTO run_steps (run_id, seq, kind, name, status, input, output, reasoning,
                       input_tokens, output_tokens, duration_ms, started_at, truncated)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13);

-- Newest first. A run's tokens include those of the runs it handed to team members.
-- name: ListRuns :many
SELECT id, agent_id, agent_name, session_id, model_provider, model, status, error,
       NULLIF(COALESCE(input_tokens, 0) + COALESCE((SELECT SUM(c.input_tokens) FROM runs c WHERE c.parent_run_id = runs.id), 0), 0),
       NULLIF(COALESCE(output_tokens, 0) + COALESCE((SELECT SUM(c.output_tokens) FROM runs c WHERE c.parent_run_id = runs.id), 0), 0),
       duration_ms, started_at, ended_at, parent_run_id
FROM runs ORDER BY started_at DESC, rowid DESC LIMIT ?1;

-- name: GetRun :one
SELECT id, agent_id, agent_name, session_id, model_provider, model, status, error,
       NULLIF(COALESCE(input_tokens, 0) + COALESCE((SELECT SUM(c.input_tokens) FROM runs c WHERE c.parent_run_id = runs.id), 0), 0),
       NULLIF(COALESCE(output_tokens, 0) + COALESCE((SELECT SUM(c.output_tokens) FROM runs c WHERE c.parent_run_id = runs.id), 0), 0),
       duration_ms, started_at, ended_at, parent_run_id
FROM runs WHERE id = ?1;

-- name: ListSteps :many
SELECT seq, kind, name, status, input, output, reasoning, input_tokens, output_tokens,
       duration_ms, started_at, truncated
FROM run_steps WHERE run_id = ?1 ORDER BY seq;

-- Runs older than the cutoff go, with their steps (cascade).
-- name: DeleteRunsBefore :exec
DELETE FROM runs WHERE started_at < ?1;

-- A run still marked running when the app starts was cut off by the app closing.
-- name: InterruptRunningRuns :exec
UPDATE runs SET status = 'error', error = 'The app was closed while this was running.', ended_at = ?1
WHERE status = 'running';

-- name: DeleteMessagesFrom :exec
DELETE FROM messages WHERE session_id = ?1 AND seq >= ?2;

-- name: InsertWorkflow :exec
INSERT INTO workflows (id, agent_id, name, goal, definition, created_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6);

-- The editor saves a workflow's name, goal and steps; its agent and runs stay.
-- name: UpdateWorkflow :exec
UPDATE workflows SET name = ?2, goal = ?3, definition = ?4 WHERE id = ?1;

-- name: ListWorkflows :many
SELECT id, agent_id, name, goal, definition, created_at FROM workflows ORDER BY name COLLATE NOCASE, rowid;

-- name: DeleteWorkflow :exec
DELETE FROM workflows WHERE id = ?1;

-- name: PutDraft :exec
INSERT INTO workflow_drafts (session_id, agent_id, name, goal, definition)
VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT (session_id) DO UPDATE SET
    agent_id = excluded.agent_id, name = excluded.name, goal = excluded.goal,
    definition = excluded.definition;

-- name: ListDrafts :many
SELECT session_id, agent_id, name, goal, definition FROM workflow_drafts;

-- name: DeleteDraft :exec
DELETE FROM workflow_drafts WHERE session_id = ?1;

-- name: StartWorkflowRun :exec
INSERT INTO workflow_runs (id, workflow_id, started_by, status, inputs, started_at)
VALUES (?1, ?2, ?3, 'running', ?4, ?5);

-- name: FinishWorkflowRun :exec
UPDATE workflow_runs SET status = ?2, output = ?3, failed_step = ?4, error = ?5, steps = ?6,
    input_tokens = ?7, output_tokens = ?8, finished_at = ?9
WHERE id = ?1;

-- Newest first. A run with a row in workflow_waits is waiting.
-- name: ListWorkflowRuns :many
SELECT r.id, r.workflow_id, r.started_by, r.status, r.inputs, r.output, r.failed_step, r.error,
       r.steps, r.input_tokens, r.output_tokens, r.started_at, r.finished_at,
       w.step, w.prompt, w.until
FROM workflow_runs r LEFT JOIN workflow_waits w ON w.run_id = r.id
WHERE r.workflow_id = ?1 ORDER BY r.started_at DESC, r.rowid DESC LIMIT ?2;

-- A run still marked running when the app starts was cut off by the app closing, unless it is
-- waiting: that is what a wait is for.
-- name: InterruptWorkflowRuns :exec
UPDATE workflow_runs SET status = 'failed', error = 'The app was closed while this was running.',
    finished_at = ?1
WHERE status = 'running' AND id NOT IN (SELECT run_id FROM workflow_waits);

-- Workflow runs older than the cutoff go with the same 30-day keep as the agents' runs. One
-- still running or waiting stays.
-- name: DeleteWorkflowRunsBefore :exec
DELETE FROM workflow_runs
WHERE started_at < ?1 AND status != 'running' AND id NOT IN (SELECT run_id FROM workflow_waits);

-- A run that is going on or waiting records how far it got, without ending.
-- name: SaveWorkflowProgress :exec
UPDATE workflow_runs SET output = ?2, steps = ?3, input_tokens = ?4, output_tokens = ?5
WHERE id = ?1;

-- name: PutWorkflowWait :exec
INSERT OR REPLACE INTO workflow_waits (run_id, step, prompt, until, snapshot, definition)
VALUES (?1, ?2, ?3, ?4, ?5, ?6);

-- name: GetWorkflowWait :one
SELECT step, prompt, until, snapshot, definition FROM workflow_waits WHERE run_id = ?1;

-- name: ClearWorkflowWait :exec
DELETE FROM workflow_waits WHERE run_id = ?1;

-- Waits whose delay is over, with the workflow they belong to.
-- name: DueWorkflowWaits :many
SELECT w.run_id, r.workflow_id FROM workflow_waits w JOIN workflow_runs r ON r.id = w.run_id
WHERE w.until IS NOT NULL AND w.until <= ?1 ORDER BY w.until;

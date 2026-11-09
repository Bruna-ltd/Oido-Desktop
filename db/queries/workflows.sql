-- name: InsertWorkflow :exec
INSERT INTO workflows (id, agent_id, name, goal, definition, created_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6);

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

-- Newest first.
-- name: ListWorkflowRuns :many
SELECT id, workflow_id, started_by, status, inputs, output, failed_step, error, steps,
       input_tokens, output_tokens, started_at, finished_at
FROM workflow_runs WHERE workflow_id = ?1 ORDER BY started_at DESC, rowid DESC LIMIT ?2;

-- A run still marked running when the app starts was cut off by the app closing.
-- name: InterruptWorkflowRuns :exec
UPDATE workflow_runs SET status = 'failed', error = 'The app was closed while this was running.',
    finished_at = ?1
WHERE status = 'running';

-- Workflow runs older than the cutoff go with the same 30-day keep as the agents' runs.
-- name: DeleteWorkflowRunsBefore :exec
DELETE FROM workflow_runs WHERE started_at < ?1 AND status != 'running';

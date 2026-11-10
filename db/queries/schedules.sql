-- name: PutSchedule :exec
INSERT INTO schedules (id, agent_id, workflow_id, name, prompt, inputs, timing, enabled, next_at, created_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
ON CONFLICT (id) DO UPDATE SET
    workflow_id = excluded.workflow_id, name = excluded.name, prompt = excluded.prompt,
    inputs = excluded.inputs, timing = excluded.timing, enabled = excluded.enabled,
    next_at = excluded.next_at;

-- name: ListSchedules :many
SELECT id, agent_id, workflow_id, name, prompt, inputs, timing, enabled, next_at, created_at
FROM schedules ORDER BY name COLLATE NOCASE, rowid;

-- After a fire, a pause or a resume.
-- name: SetScheduleState :exec
UPDATE schedules SET enabled = ?2, next_at = ?3 WHERE id = ?1;

-- name: DeleteSchedule :exec
DELETE FROM schedules WHERE id = ?1;

-- name: InsertFire :exec
INSERT INTO schedule_fires (id, schedule_id, due_at, started_at, finished_at, status, note,
                            session_id, workflow_run_id)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9);

-- name: FinishFire :exec
UPDATE schedule_fires SET status = ?2, note = ?3, finished_at = ?4, workflow_run_id = ?5
WHERE id = ?1;

-- Newest first.
-- name: ListFires :many
SELECT id, schedule_id, due_at, started_at, finished_at, status, note, session_id, workflow_run_id
FROM schedule_fires WHERE schedule_id = ?1 ORDER BY started_at DESC, rowid DESC LIMIT ?2;

-- A schedule's fires past its newest ?2, for retention. Newest first.
-- name: OldFires :many
SELECT id, session_id FROM schedule_fires WHERE schedule_id = ?1
ORDER BY started_at DESC, rowid DESC LIMIT -1 OFFSET ?2;

-- name: DeleteFire :exec
DELETE FROM schedule_fires WHERE id = ?1;

-- A fire still marked running when the app starts was cut off by the app closing.
-- name: InterruptFires :exec
UPDATE schedule_fires SET status = 'failed', note = 'The app was closed while this was running.',
    finished_at = ?1
WHERE status = 'running';

-- name: PutScheduleDraft :exec
INSERT INTO schedule_drafts (call_id, session_id, schedule_id, body) VALUES (?1, ?2, ?3, ?4)
ON CONFLICT (call_id) DO UPDATE SET
    session_id = excluded.session_id, schedule_id = excluded.schedule_id, body = excluded.body;

-- name: ListScheduleDrafts :many
SELECT call_id, session_id, schedule_id, body FROM schedule_drafts ORDER BY rowid;

-- name: DeleteScheduleDraft :exec
DELETE FROM schedule_drafts WHERE call_id = ?1;

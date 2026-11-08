-- name: ListSessions :many
SELECT id, agent_id, title FROM sessions ORDER BY agent_id, position, rowid;

-- name: UpsertSession :exec
INSERT INTO sessions (id, agent_id, title, position) VALUES (?1, ?2, ?3, ?4)
ON CONFLICT (id) DO UPDATE SET
    agent_id = excluded.agent_id, title = excluded.title, position = excluded.position,
    updated_at = unixepoch();

-- name: DeleteSession :exec
DELETE FROM sessions WHERE id = ?1;

-- name: ListMessages :many
SELECT session_id, body FROM messages ORDER BY session_id, seq;

-- name: PutMessage :exec
INSERT INTO messages (session_id, seq, body) VALUES (?1, ?2, ?3)
ON CONFLICT (session_id, seq) DO UPDATE SET body = excluded.body;

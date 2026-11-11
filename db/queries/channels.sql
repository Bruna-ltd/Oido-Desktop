-- name: GetChannelBot :one
SELECT token_enc, name, enabled FROM channel_bots WHERE channel = ?1;

-- name: UpsertChannelBot :exec
INSERT INTO channel_bots (channel, token_enc, name, enabled) VALUES (?1, ?2, ?3, ?4)
ON CONFLICT (channel) DO UPDATE SET
    token_enc = excluded.token_enc, name = excluded.name, enabled = excluded.enabled;

-- name: SetChannelBotEnabled :exec
UPDATE channel_bots SET enabled = ?2 WHERE channel = ?1;

-- name: DeleteChannelBot :exec
DELETE FROM channel_bots WHERE channel = ?1;

-- name: ListChannelUsers :many
SELECT user_id, name, handle, agent_id FROM channel_users WHERE channel = ?1 ORDER BY paired_at, user_id;

-- name: UpsertChannelUser :exec
INSERT INTO channel_users (channel, user_id, name, handle, agent_id) VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT (channel, user_id) DO UPDATE SET
    name = excluded.name, handle = excluded.handle, agent_id = excluded.agent_id;

-- name: DeleteChannelUser :exec
DELETE FROM channel_users WHERE channel = ?1 AND user_id = ?2;

-- name: ListChannelChats :many
SELECT user_id, agent_id, session_id FROM channel_chats WHERE channel = ?1;

-- name: UpsertChannelChat :exec
INSERT INTO channel_chats (channel, user_id, agent_id, session_id) VALUES (?1, ?2, ?3, ?4)
ON CONFLICT (channel, user_id, agent_id) DO UPDATE SET session_id = excluded.session_id;

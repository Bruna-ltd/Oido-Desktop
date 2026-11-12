-- Top-level agents first, then team members, each in their saved order.
-- name: ListAgents :many
SELECT id, parent_id, name, prompt, model, hue, pinned, skills, tools, active_session, model_provider, workdir, description, trust_workdir, channel_topics, avatar_preset, avatar_image
FROM agents ORDER BY parent_id IS NOT NULL, position, rowid;

-- name: UpsertAgent :exec
INSERT INTO agents (id, parent_id, name, prompt, model, hue, pinned, skills, tools, active_session, position, model_provider, workdir, description, trust_workdir, channel_topics, avatar_preset, avatar_image)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
ON CONFLICT (id) DO UPDATE SET
    parent_id = excluded.parent_id, name = excluded.name, prompt = excluded.prompt,
    model = excluded.model, hue = excluded.hue, pinned = excluded.pinned,
    skills = excluded.skills, tools = excluded.tools,
    active_session = excluded.active_session, position = excluded.position,
    model_provider = excluded.model_provider, workdir = excluded.workdir, description = excluded.description,
    trust_workdir = excluded.trust_workdir, channel_topics = excluded.channel_topics,
    avatar_preset = excluded.avatar_preset, avatar_image = excluded.avatar_image,
    updated_at = unixepoch();

-- name: DeleteAgent :exec
DELETE FROM agents WHERE id = ?1;

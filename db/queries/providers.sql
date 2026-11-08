-- name: ListProviders :many
SELECT id, label, custom, connected, key_hint, base_url FROM providers ORDER BY position, rowid;

-- name: UpsertProvider :exec
INSERT INTO providers (id, label, custom, connected, key_hint, base_url, position)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT (id) DO UPDATE SET
    label = excluded.label, custom = excluded.custom, connected = excluded.connected,
    key_hint = excluded.key_hint, base_url = excluded.base_url, position = excluded.position,
    -- A provider saved as disconnected holds no key: signing out forgets it, in the same statement.
    api_key_enc = CASE WHEN excluded.connected = 0 THEN NULL ELSE providers.api_key_enc END,
    updated_at = unixepoch();

-- name: DeleteProvider :exec
DELETE FROM providers WHERE id = ?1;

-- The sealed key, or NULL. ListProviders never selects it: a key is only read when asked for.
-- name: GetProviderKey :one
SELECT api_key_enc FROM providers WHERE id = ?1;

-- name: SetProviderKey :exec
UPDATE providers SET api_key_enc = ?2, updated_at = unixepoch() WHERE id = ?1;

-- Built-in providers that say they are connected but have no key: a connect that was cut short.
-- name: ListConnectedWithoutKey :many
SELECT id FROM providers WHERE custom = 0 AND connected = 1 AND api_key_enc IS NULL;

-- name: ListModels :many
SELECT provider_id, model_id, name, vision, reasoning, tools FROM models ORDER BY provider_id, position;

-- name: ListModelFetchTimes :many
SELECT id, models_fetched_at FROM providers WHERE models_fetched_at IS NOT NULL;

-- name: DeleteModels :exec
DELETE FROM models WHERE provider_id = ?1;

-- name: InsertModel :exec
INSERT INTO models (provider_id, model_id, name, vision, reasoning, tools, position)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7);

-- name: SetModelsFetchedAt :exec
UPDATE providers SET models_fetched_at = ?2 WHERE id = ?1;

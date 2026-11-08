-- name: GetSetting :one
SELECT value FROM settings WHERE key = ?1;

-- name: SetSetting :exec
INSERT INTO settings (key, value) VALUES (?1, ?2)
ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = unixepoch();

-- name: DeleteSetting :exec
DELETE FROM settings WHERE key = ?1;

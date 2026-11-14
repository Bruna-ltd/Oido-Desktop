-- name: ListExtensions :many
SELECT id, enabled, off_tools FROM extensions;

-- name: SaveExtension :exec
INSERT INTO extensions (id, enabled, off_tools) VALUES (?1, ?2, ?3)
ON CONFLICT (id) DO UPDATE SET enabled = excluded.enabled, off_tools = excluded.off_tools;

-- ExtensionAccount is read only when a call or a screen needs it opened.
-- name: GetExtensionAccount :one
SELECT secrets_enc FROM extension_accounts WHERE provider = ?1;

-- name: SaveExtensionAccount :exec
INSERT INTO extension_accounts (provider, secrets_enc) VALUES (?1, ?2)
ON CONFLICT (provider) DO UPDATE SET secrets_enc = excluded.secrets_enc;

-- name: DeleteExtensionAccount :exec
DELETE FROM extension_accounts WHERE provider = ?1;

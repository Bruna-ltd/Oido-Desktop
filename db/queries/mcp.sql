-- ListMcpServers never selects the secrets: they are only read when a call needs them.
-- name: ListMcpServers :many
SELECT id, name, transport, url, command, args, auth, enabled, status, status_detail, tools
FROM mcp_servers ORDER BY position, rowid;

-- name: InsertMcpServer :exec
INSERT INTO mcp_servers (id, name, transport, url, command, args, auth, enabled, status, status_detail, tools, secrets_enc, position)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
        (SELECT COALESCE(MAX(position), -1) + 1 FROM mcp_servers));

-- name: DeleteMcpServer :exec
DELETE FROM mcp_servers WHERE id = ?1;

-- name: SetMcpEnabled :exec
UPDATE mcp_servers SET enabled = ?2 WHERE id = ?1;

-- name: UpdateMcpServer :exec
UPDATE mcp_servers SET transport = ?2, url = ?3, command = ?4, args = ?5, auth = ?6 WHERE id = ?1;

-- name: SetMcpListing :exec
UPDATE mcp_servers SET status = ?2, status_detail = ?3, tools = ?4 WHERE id = ?1;

-- name: GetMcpSecrets :one
SELECT secrets_enc FROM mcp_servers WHERE id = ?1;

-- name: SetMcpSecrets :exec
UPDATE mcp_servers SET secrets_enc = ?2 WHERE id = ?1;

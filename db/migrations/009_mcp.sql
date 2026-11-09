-- MCP servers: other programs that offer tools to agents. A server is remote (a URL) or local (a
-- command the app starts). What it offered when last asked is kept as JSON, so an agent can be given
-- its tools without the server being reached. Its key, the variables a local server is started with
-- and its sign-in tokens are one sealed value (src/vault.rs), never readable text in this file.
CREATE TABLE mcp_servers (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE CHECK (length(trim(name)) > 0),
    transport     TEXT NOT NULL CHECK (transport IN ('http', 'stdio')),
    url           TEXT NOT NULL DEFAULT '',
    command       TEXT NOT NULL DEFAULT '',
    args          TEXT NOT NULL DEFAULT '[]',
    auth          TEXT NOT NULL DEFAULT 'none' CHECK (auth IN ('none', 'key', 'oauth')),
    enabled       INTEGER NOT NULL DEFAULT 1,
    status        TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'reachable', 'unreachable')),
    status_detail TEXT NOT NULL DEFAULT '',
    tools         TEXT NOT NULL DEFAULT '[]',
    secrets_enc   BLOB,
    position      INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

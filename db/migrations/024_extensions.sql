-- Extensions: tool sets built into the app (Gmail, Calendar, Drive) that agents get as MCP tools.
-- A row says whether one is on and which of its tools the person switched off. What they are
-- signed in with is one sealed value per provider (src/vault.rs): the client the person made there
-- and the sign-in tokens, never readable text in this file.
CREATE TABLE extensions (
    id        TEXT PRIMARY KEY,
    enabled   INTEGER NOT NULL DEFAULT 0,
    off_tools TEXT NOT NULL DEFAULT '[]'
) STRICT;

CREATE TABLE extension_accounts (
    provider    TEXT PRIMARY KEY,
    secrets_enc BLOB NOT NULL
) STRICT;

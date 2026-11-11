-- Slack joins Telegram as a channel. The channel names were checked by the schema, which SQLite
-- cannot change in place; the tables are rebuilt without that check (the app writes only names it
-- knows), with the same rows.
--
-- Dropping `channel_users` would delete the chats bound to it (the foreign key cascades), so those
-- are kept aside first and put back once the people are.

CREATE TABLE channel_bots_new (
    channel   TEXT PRIMARY KEY,
    token_enc BLOB NOT NULL,
    name      TEXT NOT NULL,
    enabled   INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))
) STRICT;
INSERT INTO channel_bots_new SELECT channel, token_enc, name, enabled FROM channel_bots;
DROP TABLE channel_bots;
ALTER TABLE channel_bots_new RENAME TO channel_bots;

CREATE TABLE channel_chats_kept AS SELECT channel, user_id, agent_id, session_id FROM channel_chats;
DROP TABLE channel_chats;

CREATE TABLE channel_users_new (
    channel   TEXT NOT NULL,
    user_id   TEXT NOT NULL,
    name      TEXT NOT NULL,
    handle    TEXT,
    agent_id  TEXT REFERENCES agents (id) ON DELETE SET NULL,
    paired_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (channel, user_id)
) STRICT;
INSERT INTO channel_users_new SELECT channel, user_id, name, handle, agent_id, paired_at FROM channel_users;
DROP TABLE channel_users;
ALTER TABLE channel_users_new RENAME TO channel_users;

CREATE TABLE channel_chats (
    channel    TEXT NOT NULL,
    user_id    TEXT NOT NULL,
    agent_id   TEXT NOT NULL REFERENCES agents (id) ON DELETE CASCADE,
    session_id TEXT NOT NULL UNIQUE REFERENCES sessions (id) ON DELETE CASCADE,
    PRIMARY KEY (channel, user_id, agent_id),
    FOREIGN KEY (channel, user_id) REFERENCES channel_users (channel, user_id) ON DELETE CASCADE
) STRICT;
INSERT INTO channel_chats SELECT channel, user_id, agent_id, session_id FROM channel_chats_kept;
DROP TABLE channel_chats_kept;

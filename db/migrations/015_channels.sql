-- Channels: messaging apps (Telegram first) through which paired people talk to agents. See
-- docs/channels.md.

-- A connected bot. Its token is sealed by the app (src/vault.rs), bound to the channel's name.
CREATE TABLE channel_bots (
    channel   TEXT PRIMARY KEY CHECK (channel IN ('telegram')),
    token_enc BLOB NOT NULL,
    -- The bot's handle, as the app shows it (`oido_bot`).
    name      TEXT NOT NULL,
    enabled   INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))
) STRICT;

-- The people allowed to talk to agents, approved in Settings: everyone else is ignored. The
-- agent is the one their next message goes to.
CREATE TABLE channel_users (
    channel   TEXT NOT NULL CHECK (channel IN ('telegram')),
    user_id   TEXT NOT NULL,
    name      TEXT NOT NULL,
    handle    TEXT,
    agent_id  TEXT REFERENCES agents (id) ON DELETE SET NULL,
    paired_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (channel, user_id)
) STRICT;

-- The chat each person carries on with each agent. A chat or agent deleted, or a person
-- unpaired, ends the binding; the chat itself stays.
CREATE TABLE channel_chats (
    channel    TEXT NOT NULL,
    user_id    TEXT NOT NULL,
    agent_id   TEXT NOT NULL REFERENCES agents (id) ON DELETE CASCADE,
    session_id TEXT NOT NULL UNIQUE REFERENCES sessions (id) ON DELETE CASCADE,
    PRIMARY KEY (channel, user_id, agent_id),
    FOREIGN KEY (channel, user_id) REFERENCES channel_users (channel, user_id) ON DELETE CASCADE
) STRICT;

-- Whether people on channels may pick the agent.
ALTER TABLE agents ADD COLUMN on_channels INTEGER NOT NULL DEFAULT 0 CHECK (on_channels IN (0, 1));

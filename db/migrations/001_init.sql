-- The desktop app's own local store. Secrets are not kept here: a provider's key lives in the
-- OS keyring and only a masked hint is stored.

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL CHECK (json_valid(value)),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE TABLE providers (
    id         TEXT PRIMARY KEY,
    label      TEXT NOT NULL,
    custom     INTEGER NOT NULL DEFAULT 0 CHECK (custom IN (0, 1)),
    connected  INTEGER NOT NULL DEFAULT 0 CHECK (connected IN (0, 1)),
    key_hint   TEXT,
    base_url   TEXT,
    position   INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE TABLE agents (
    id             TEXT PRIMARY KEY,
    parent_id      TEXT REFERENCES agents (id) ON DELETE CASCADE,
    name           TEXT NOT NULL CHECK (length(trim(name)) > 0),
    prompt         TEXT NOT NULL DEFAULT '',
    model          TEXT,
    hue            REAL,
    pinned         INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
    skills         TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(skills)),
    tools          TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tools)),
    active_session INTEGER NOT NULL DEFAULT 0,
    position       INTEGER NOT NULL DEFAULT 0,
    updated_at     INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX agents_by_parent ON agents (parent_id, position);

-- Two levels at most: a team member cannot have a team of its own, and an agent that has a team
-- cannot become a member.
CREATE TRIGGER agents_two_levels_on_insert
BEFORE INSERT ON agents
WHEN NEW.parent_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a team member cannot have team members')
    WHERE EXISTS (SELECT 1 FROM agents WHERE id = NEW.parent_id AND parent_id IS NOT NULL);
END;

CREATE TRIGGER agents_two_levels_on_update
BEFORE UPDATE OF parent_id ON agents
WHEN NEW.parent_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'a team member cannot have team members')
    WHERE EXISTS (SELECT 1 FROM agents WHERE id = NEW.parent_id AND parent_id IS NOT NULL)
       OR EXISTS (SELECT 1 FROM agents WHERE parent_id = NEW.id);
END;

CREATE TABLE sessions (
    id         TEXT PRIMARY KEY,
    agent_id   TEXT NOT NULL REFERENCES agents (id) ON DELETE CASCADE,
    title      TEXT NOT NULL DEFAULT 'New chat',
    position   INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
) STRICT;

CREATE INDEX sessions_by_agent ON sessions (agent_id, position);

-- A conversation, one JSON document per entry (the `Message` type, tagged by `type`).
CREATE TABLE messages (
    session_id TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    body       TEXT NOT NULL CHECK (json_valid(body)),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (session_id, seq)
) STRICT, WITHOUT ROWID;

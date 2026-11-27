-- What each connected channel heard: every message it was sent or saw, from anyone, in any chat
-- it is in (on WhatsApp, every chat of the linked number). Words only; a file is noted by its
-- name. Kept 30 days, for agents to read with `read_messages`. See docs/channels.md.
CREATE TABLE channel_log (
    id          INTEGER PRIMARY KEY,
    channel     TEXT NOT NULL,
    -- The chat it was in, as the channel names it, and its title where it has one (a group's).
    chat        TEXT NOT NULL,
    chat_name   TEXT,
    sender      TEXT NOT NULL,
    sender_name TEXT NOT NULL,
    -- When it was sent, in seconds since the epoch.
    at          INTEGER NOT NULL,
    text        TEXT NOT NULL,
    file        TEXT,
    -- The message's own id, where the channel has one: the same message heard twice is kept once.
    msg_id      TEXT
) STRICT;

CREATE UNIQUE INDEX channel_log_message ON channel_log (channel, chat, msg_id);
CREATE INDEX channel_log_at ON channel_log (channel, at);

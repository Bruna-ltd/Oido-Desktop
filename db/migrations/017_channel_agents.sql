-- Which agents people on each channel may pick, chosen per channel on the Channels screen, in
-- place of one switch per agent for every channel at once. Agents switched on keep being
-- reachable on each channel that has a bot; `agents.on_channels` is no longer read.
CREATE TABLE channel_agents (
    channel  TEXT NOT NULL,
    agent_id TEXT NOT NULL REFERENCES agents (id) ON DELETE CASCADE,
    PRIMARY KEY (channel, agent_id)
) STRICT, WITHOUT ROWID;

INSERT INTO channel_agents (channel, agent_id)
SELECT b.channel, a.id FROM channel_bots b, agents a
WHERE a.on_channels = 1 AND a.parent_id IS NULL;

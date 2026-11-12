-- What an agent answers about on channels (its form, as a JSON list of words or topics): a message
-- from a channel that names none of them is turned away before it reaches the agent. Empty, as it
-- starts, it answers anything. See docs/guardrails.md.
ALTER TABLE agents ADD COLUMN channel_topics TEXT NOT NULL DEFAULT '[]';

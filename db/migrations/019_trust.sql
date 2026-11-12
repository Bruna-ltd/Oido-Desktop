-- Taint (docs/guardrails.md): whether what an agent reads and runs in its working folder counts as
-- the person's own (on unless turned off), and whether a paired person's messages do (off unless
-- turned on). Text that does not is outside text: while it is in what a model is sent, the agent's
-- calls that act wait for a go-ahead.
ALTER TABLE agents ADD COLUMN trust_workdir INTEGER NOT NULL DEFAULT 1 CHECK (trust_workdir IN (0, 1));
ALTER TABLE channel_users ADD COLUMN trusted INTEGER NOT NULL DEFAULT 0 CHECK (trusted IN (0, 1));
